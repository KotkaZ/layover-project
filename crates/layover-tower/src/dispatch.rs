//! Deciding whether a flight may fly, and what starting it would mean.
//!
//! # Why this is separate from spawning
//!
//! Everything here is a refusal that should happen *before* a process exists. A flight down an
//! edge the route map does not permit, a chain with no Hops left, an itinerary out of Fuel — each
//! of those is cheaper to catch now than after a CLI has started and begun spending.
//!
//! Keeping the decision apart from the act also means it can be tested exhaustively without a
//! process in sight, which matters because this is where the safety rails actually bite. A rail
//! that is only exercised through a real spawn is a rail tested a handful of times.
//!
//! # The order the checks run in
//!
//! Ground Stop, then route, then rails. Not arbitrary:
//!
//! - **Ground Stop first**, because when everything is meant to have stopped, the reason a flight
//!   was refused should be "everything is stopped" and not a detail about that particular flight.
//! - **Route before rails**, because "that edge does not exist" is a fact about the factory's
//!   definition and will be true on every attempt, whereas "no Fuel left" is a fact about this
//!   chain right now. Reporting the permanent problem first saves somebody re-triggering work
//!   that was never going to be permitted.

use std::collections::BTreeMap;

use layover_core::agent::{Agent, AgentName};
use layover_core::config::Config;
use layover_core::flight::Flight;
use layover_core::graph::RouteGraph;
use layover_core::itinerary::{Denial, Itinerary};

/// Why a flight will not be flown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A Ground Stop is engaged, so nothing starts.
    GroundStop,
    /// The flight names an agent the factory does not declare.
    NoSuchAgent {
        /// The name that is not declared.
        agent: AgentName,
    },
    /// The route map does not permit this edge.
    ///
    /// Not an error in the sending agent so much as a fact about the factory: the mesh is a
    /// permission graph, and an edge that is not drawn is a message that may not be sent.
    NoRoute {
        /// Who tried to send.
        from: AgentName,
        /// Who they tried to reach.
        to: AgentName,
    },
    /// The agent names a runner the factory does not declare.
    NoSuchRunner {
        /// The agent whose runner is missing.
        agent: AgentName,
        /// The runner that is not declared.
        runner: String,
    },
    /// The agent declares no runner and there is no default.
    NoRunner {
        /// The agent with nothing to run it.
        agent: AgentName,
    },
    /// A safety rail refused it.
    Rail(Denial),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GroundStop => f.write_str("a Ground Stop is engaged, so nothing new starts"),
            Self::NoSuchAgent { agent } => write!(f, "no agent called `{agent}` is declared"),
            Self::NoRoute { from, to } => write!(
                f,
                "the route map does not permit `{from}` to send to `{to}`"
            ),
            Self::NoSuchRunner { agent, runner } => write!(
                f,
                "agent `{agent}` names runner `{runner}`, which is not declared"
            ),
            Self::NoRunner { agent } => write!(
                f,
                "agent `{agent}` declares no runner and there is no `[defaults] runner`"
            ),
            Self::Rail(denial) => write!(f, "{denial}"),
        }
    }
}

impl std::error::Error for Refusal {}

/// A flight that may fly, and what it costs the chain.
#[derive(Debug, Clone)]
pub struct Authorised<'a> {
    /// The agent that will run.
    pub agent: &'a Agent,
    /// Its name.
    pub name: AgentName,
    /// Hops remaining for the run this starts.
    ///
    /// Already decremented: this is what the *next* flight from this agent will be authorised
    /// against, and carrying it here is what stops an agent choosing its own depth budget.
    pub hops_remaining: u32,
    /// Which runner will invoke it.
    pub runner: String,
}

/// Decides whether `flight` may be flown, and against which agent.
///
/// `sender` is `None` for a flight from a human — there is no upstream agent, so there is no edge
/// to check. The rails still apply: a human trigger spends Hops and Fuel like anything else.
///
/// # Errors
///
/// Returns the first [`Refusal`] that applies, in the order documented on this module.
pub fn authorise<'a>(
    config: &'a Config,
    graph: &RouteGraph,
    itinerary: &Itinerary,
    sender: Option<&AgentName>,
    flight: &Flight,
    ground_stop_engaged: bool,
) -> Result<Authorised<'a>, Refusal> {
    if ground_stop_engaged {
        return Err(Refusal::GroundStop);
    }

    let name = flight.to.clone();
    let agent = config
        .agents
        .get(&name)
        .ok_or_else(|| Refusal::NoSuchAgent {
            agent: name.clone(),
        })?;

    if let Some(from) = sender
        && !graph.permits(from, &name)
    {
        return Err(Refusal::NoRoute {
            from: from.clone(),
            to: name.clone(),
        });
    }

    // Hops are checked against what the *sender's* run had left, which the flight carries. An
    // agent cannot award itself more depth than it was given, because the number never passes
    // through the agent — the Tower reads it from the flight it minted.
    let hops_remaining = itinerary
        .authorize_send(flight.hops_remaining)
        .map_err(Refusal::Rail)?;

    let runner = agent
        .runner
        .clone()
        .or_else(|| config.defaults.runner.clone())
        .ok_or_else(|| Refusal::NoRunner {
            agent: name.clone(),
        })?;

    if !config.runners.contains_key(&runner) {
        return Err(Refusal::NoSuchRunner {
            agent: name.clone(),
            runner,
        });
    }

    Ok(Authorised {
        agent,
        name,
        hops_remaining,
        runner,
    })
}

/// Resolves the environment an agent needs: its own CLI's credentials and its MCP servers'.
///
/// Collected across every server the agent declares, because they are all started inside the one
/// child process and share its environment, and combined with what the agent named for itself.
/// `defaults` covers the credential every agent's CLI needs; it adds to the agent's own list
/// rather than being overridden by it, because the two answer different questions — "what does
/// this CLI need to start" and "what may this particular agent hold".
#[must_use]
pub fn declared_env(agent: &Agent, defaults: &[String]) -> Vec<String> {
    let mut names: Vec<String> = agent
        .mcp
        .values()
        .flat_map(|server| server.env_from.iter().cloned())
        .chain(agent.env_from.iter().cloned())
        .chain(defaults.iter().cloned())
        .collect();

    names.sort();
    names.dedup();
    names
}

/// Explicit values an agent's MCP servers set, as opposed to names they read from the environment.
#[must_use]
pub fn declared_values(agent: &Agent) -> BTreeMap<String, String> {
    agent
        .mcp
        .values()
        .flat_map(|server| server.env.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use layover_core::flight::{ItineraryId, Origin};

    const FACTORY: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "claude"
max_hops = 4
fuel_usd = 5.0
max_runs = 10

[runners.claude]
command = ["claude", "-p"]

[agents.analyst]
prompt = "analyse"
entry = true

[agents.developer]
prompt = "develop"

[agents.stranger]
prompt = "lurk"

[pipelines.build]
entry = "analyst"

[[routes]]
from = "analyst"
to = "developer"
"#;

    fn factory() -> (Config, RouteGraph) {
        let config: Config = toml::from_str(FACTORY).expect("parses");
        let graph = RouteGraph::from_config(&config);
        (config, graph)
    }

    fn itinerary() -> Itinerary {
        Itinerary::new(ItineraryId::generate(), 4, 5.0, 10)
    }

    fn flight_to(to: &str, hops: u32) -> Flight {
        Flight::new(
            ItineraryId::generate(),
            Origin::Human,
            AgentName::new(to),
            "do the thing",
            hops,
        )
    }

    #[test]
    fn a_human_trigger_needs_no_edge() {
        // There is no upstream agent, so there is nothing to check an edge against. The rails
        // still apply.
        let (config, graph) = factory();
        let authorised = authorise(
            &config,
            &graph,
            &itinerary(),
            None,
            &flight_to("analyst", 4),
            false,
        )
        .expect("a human may trigger an agent");

        assert_eq!(authorised.name, AgentName::new("analyst"));
        assert_eq!(authorised.runner, "claude");
    }

    #[test]
    fn a_permitted_edge_is_flown() {
        let (config, graph) = factory();
        let sender = AgentName::new("analyst");

        assert!(
            authorise(
                &config,
                &graph,
                &itinerary(),
                Some(&sender),
                &flight_to("developer", 4),
                false
            )
            .is_ok()
        );
    }

    #[test]
    fn an_edge_the_map_does_not_draw_is_refused() {
        // The mesh is a permission graph. An edge that is not drawn is a message that may not be
        // sent, however reasonable it looks.
        let (config, graph) = factory();
        let sender = AgentName::new("developer");

        let refusal = authorise(
            &config,
            &graph,
            &itinerary(),
            Some(&sender),
            &flight_to("analyst", 4),
            false,
        )
        .expect_err("developer may not send to analyst");

        assert_eq!(
            refusal,
            Refusal::NoRoute {
                from: AgentName::new("developer"),
                to: AgentName::new("analyst"),
            }
        );
    }

    #[test]
    fn a_ground_stop_refuses_before_anything_else_is_considered() {
        // When everything is meant to have stopped, the reason should be that everything is
        // stopped -- not a detail about this particular flight.
        let (config, graph) = factory();
        let sender = AgentName::new("developer");

        let refusal = authorise(
            &config,
            &graph,
            &itinerary(),
            Some(&sender),
            &flight_to("nonexistent", 0),
            true,
        )
        .expect_err("a Ground Stop refuses everything");

        assert_eq!(refusal, Refusal::GroundStop);
    }

    #[test]
    fn a_chain_out_of_hops_is_cut() {
        let (config, graph) = factory();

        let refusal = authorise(
            &config,
            &graph,
            &itinerary(),
            None,
            &flight_to("analyst", 0),
            false,
        )
        .expect_err("zero hops ends the chain");

        assert_eq!(refusal, Refusal::Rail(Denial::HopsExhausted));
    }

    #[test]
    fn hops_come_back_decremented_so_an_agent_cannot_award_itself_more() {
        // The count never passes through the agent: the Tower reads it from the flight it minted
        // and hands the run what is left.
        let (config, graph) = factory();
        let authorised = authorise(
            &config,
            &graph,
            &itinerary(),
            None,
            &flight_to("analyst", 3),
            false,
        )
        .expect("authorised");

        assert_eq!(authorised.hops_remaining, 2);
    }

    #[test]
    fn an_exhausted_itinerary_refuses_on_the_rail_not_the_route() {
        let (config, graph) = factory();
        let mut spent = itinerary();
        spent.debit_fuel(99.0);

        let refusal = authorise(
            &config,
            &graph,
            &spent,
            None,
            &flight_to("analyst", 4),
            false,
        )
        .expect_err("no Fuel left");

        assert_eq!(refusal, Refusal::Rail(Denial::FuelExhausted));
    }

    #[test]
    fn an_undeclared_agent_is_refused_by_name() {
        let (config, graph) = factory();

        let refusal = authorise(
            &config,
            &graph,
            &itinerary(),
            None,
            &flight_to("ghost", 4),
            false,
        )
        .expect_err("no such agent");

        assert!(matches!(refusal, Refusal::NoSuchAgent { .. }));
        assert!(refusal.to_string().contains("ghost"), "{refusal}");
    }

    #[test]
    fn a_route_problem_is_reported_before_a_rail_problem() {
        // "That edge does not exist" is true on every attempt; "no Fuel left" is true of this
        // chain right now. Reporting the permanent one first saves re-triggering work that was
        // never going to be permitted.
        let (config, graph) = factory();
        let mut spent = itinerary();
        spent.debit_fuel(99.0);
        let sender = AgentName::new("developer");

        let refusal = authorise(
            &config,
            &graph,
            &spent,
            Some(&sender),
            &flight_to("analyst", 4),
            false,
        )
        .expect_err("refused");

        assert!(
            matches!(refusal, Refusal::NoRoute { .. }),
            "got {refusal:?}, expected the route problem to win"
        );
    }

    #[test]
    fn an_agent_with_no_runner_anywhere_is_refused() {
        let text = FACTORY.replace("runner = \"claude\"\n", "");
        let config: Config = toml::from_str(&text).expect("parses");
        let graph = RouteGraph::from_config(&config);

        let refusal = authorise(
            &config,
            &graph,
            &itinerary(),
            None,
            &flight_to("analyst", 4),
            false,
        )
        .expect_err("nothing to run it");

        assert!(matches!(refusal, Refusal::NoRunner { .. }), "{refusal:?}");
    }

    #[test]
    fn credentials_are_gathered_from_every_declared_server_without_duplicates() {
        let text = r#"
[layover]
work_dir = "work"

[defaults]
runner = "claude"

[runners.claude]
command = ["claude", "-p"]

[agents.analyst]
prompt = "analyse"
entry = true

[agents.analyst.mcp.one]
url = "https://example.invalid/"
env_from = ["SHARED_TOKEN", "ONE_TOKEN"]

[agents.analyst.mcp.two]
url = "https://example.invalid/"
env_from = ["SHARED_TOKEN", "TWO_TOKEN"]
"#;
        let config: Config = toml::from_str(text).expect("parses");
        let agent = config
            .agents
            .get(&AgentName::new("analyst"))
            .expect("declared");

        assert_eq!(
            declared_env(agent, &[]),
            vec!["ONE_TOKEN", "SHARED_TOKEN", "TWO_TOKEN"],
            "a name declared twice is still one variable"
        );
    }

    #[test]
    fn an_agent_can_name_its_own_cli_credentials_alongside_its_servers() {
        // The agent CLI needs a credential before it can do anything at all, and it is not the
        // same credential its MCP servers need. Without this the child authenticates as nobody
        // and the run dies before it reads its instructions.
        let text = r#"
[agents.analyst]
prompt = "go"
env_from = ["GITHUB_TOKEN"]

[agents.analyst.mcp.kusto]
url = "https://example.invalid/"
env_from = ["KUSTO_TOKEN"]
"#;
        let config: Config = toml::from_str(text).expect("parses");
        let agent = config
            .agents
            .get(&AgentName::new("analyst"))
            .expect("declared");

        assert_eq!(
            declared_env(agent, &[]),
            vec!["GITHUB_TOKEN", "KUSTO_TOKEN"]
        );
    }

    #[test]
    fn defaults_add_to_an_agents_own_names_rather_than_replacing_them() {
        // One CLI credential shared by every agent, plus whatever this one alone may hold. If
        // defaults were overridden, naming a private token would silently drop the shared one and
        // the agent would fail to authenticate.
        let text = r#"
[defaults]
env_from = ["GITHUB_TOKEN"]

[agents.publisher]
prompt = "go"
env_from = ["RELEASE_TOKEN"]

[agents.reader]
prompt = "go"
"#;
        let config: Config = toml::from_str(text).expect("parses");
        let defaults = &config.defaults.env_from;

        let publisher = config
            .agents
            .get(&AgentName::new("publisher"))
            .expect("declared");
        assert_eq!(
            declared_env(publisher, defaults),
            vec!["GITHUB_TOKEN", "RELEASE_TOKEN"]
        );

        // And the agent that named nothing still gets the shared one.
        let reader = config
            .agents
            .get(&AgentName::new("reader"))
            .expect("declared");
        assert_eq!(declared_env(reader, defaults), vec!["GITHUB_TOKEN"]);

        // The publishing token stays with the publisher.
        assert!(!declared_env(reader, defaults).contains(&"RELEASE_TOKEN".to_owned()));
    }
}
