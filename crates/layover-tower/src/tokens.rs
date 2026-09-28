//! Per-run tokens: the identity a child proves itself with.
//!
//! The configuration file that tells a child where to find Layover is written by
//! [`crate::mcp_config`].
//!
//! # Why the token is the identity
//!
//! A child process is untrusted. It is an LLM that has read a work item, a pull request comment
//! and possibly another agent's output, any of which may be trying to talk it into something. So
//! nothing it *says* about itself can be believed — including, especially, which agent it is.
//!
//! The token closes that. It is random, minted per run, and never derived from the agent's name;
//! the Tower keeps the mapping and looks the caller up. An agent cannot name itself because there
//! is no field in which to do so, and every rail in the system — Hops, Fuel, the run cap, who may
//! send to whom — is indexed by the name only the Tower holds.
//!
//! # Why it dies with the run
//!
//! A token that outlives its run is a way for a finished process to keep sending flights. Runs
//! are the unit everything is accounted against, so a call arriving after its run has ended has no
//! itinerary to charge and no agent to be — it is refused, and refused loudly, because it means
//! either a bug or a child that outlived its supervisor.

use std::collections::HashMap;
use std::sync::Mutex;

use layover_core::agent::AgentName;
use layover_core::flight::{ItineraryId, RunId};
use layover_mcp::Session;

/// The environment variable a child finds its token in.
pub const TOKEN_VAR: &str = "LAYOVER_RUN_TOKEN";

/// The environment variable a child finds Layover's MCP endpoint in.
pub const ENDPOINT_VAR: &str = "LAYOVER_MCP_URL";

/// Tokens the Tower has minted and not yet revoked.
#[derive(Debug, Default)]
pub struct Tokens {
    live: Mutex<HashMap<String, Session>>,
}

impl Tokens {
    /// A registry with nothing in it.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mints a token for a run, returning it.
    ///
    /// The value is random rather than derived: a token computable from the run or agent name
    /// would let anything that could guess a name mint its own.
    pub fn mint(
        &self,
        run: RunId,
        agent: AgentName,
        itinerary: ItineraryId,
        hops_remaining: u32,
    ) -> String {
        self.mint_for(Session {
            run,
            agent,
            itinerary,
            hops_remaining,
            pipeline: None,
            flags: std::collections::BTreeMap::new(),
        })
    }

    /// Mints a token for a run whose identity, chain and flags the Tower has already assembled.
    pub fn mint_for(&self, session: Session) -> String {
        let token = format!("lvt_{}", RunId::generate().as_str().replace("run_", ""));

        if let Ok(mut live) = self.live.lock() {
            live.insert(token.clone(), session);
        }

        token
    }

    /// Who a token belongs to, if it is still live.
    #[must_use]
    pub fn resolve(&self, token: &str) -> Option<Session> {
        self.live.lock().ok()?.get(token).cloned()
    }

    /// Ends a token's life.
    ///
    /// Called when a run finishes, times out, crashes, or is stopped. A token that outlives its
    /// run is a finished process that can still send work.
    pub fn revoke(&self, token: &str) {
        if let Ok(mut live) = self.live.lock() {
            live.remove(token);
        }
    }

    /// How many tokens are live, for reporting.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live.lock().map_or(0, |live| live.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> Tokens {
        Tokens::new()
    }

    fn mint(registry: &Tokens, agent: &str) -> String {
        registry.mint(
            RunId::generate(),
            AgentName::new(agent),
            ItineraryId::generate(),
            3,
        )
    }

    #[test]
    fn a_token_resolves_to_the_run_that_holds_it() {
        let registry = tokens();
        let token = mint(&registry, "analyst");

        let session = registry.resolve(&token).expect("live");
        assert_eq!(session.agent, AgentName::new("analyst"));
        assert_eq!(session.hops_remaining, 3);
    }

    #[test]
    fn a_token_is_not_derivable_from_the_agent_it_belongs_to() {
        // A token computable from a name would let anything that could guess a name mint its own.
        let registry = tokens();
        let first = mint(&registry, "analyst");
        let second = mint(&registry, "analyst");

        assert_ne!(
            first, second,
            "two runs of one agent must not share a token"
        );
        assert!(!first.contains("analyst"));
    }

    #[test]
    fn an_unknown_token_resolves_to_nobody() {
        assert!(tokens().resolve("lvt_invented").is_none());
    }

    #[test]
    fn a_revoked_token_stops_working_immediately() {
        // This is what stops a finished process from still sending flights.
        let registry = tokens();
        let token = mint(&registry, "analyst");

        registry.revoke(&token);

        assert!(registry.resolve(&token).is_none());
        assert_eq!(registry.live_count(), 0);
    }

    #[test]
    fn revoking_one_run_does_not_disturb_another() {
        let registry = tokens();
        let first = mint(&registry, "analyst");
        let second = mint(&registry, "developer");

        registry.revoke(&first);

        assert!(registry.resolve(&second).is_some());
    }
}
