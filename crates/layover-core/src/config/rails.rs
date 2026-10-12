//! The rails one chain runs under: how deep it may go, what it may spend, how many runs it may
//! start, and how long each may take.
//!
//! `[defaults]` sets them for the factory. A workflow may set its own in `[pipelines.<name>]`,
//! because one set rarely fits both a development chain that runs for an afternoon and a review a
//! sweep spawns per pull request — a factory sized for the first leaves the second nearly
//! unbounded. An agent's own `fuel_usd` is the Fuel of a chain that starts at it.
//!
//! Who wins, for each: the chain's workflow, then — for Fuel — the agent the chain starts at, then
//! `[defaults]`. The Reserve is not here: it bounds the whole factory, whatever its workflows say.

use super::Config;
use crate::agent::AgentName;
use crate::pipeline::PipelineName;

/// The rails of one chain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rails {
    /// Flights the chain may make before it is cut.
    pub max_hops: u32,
    /// Dollars the chain may spend.
    pub fuel_usd: f64,
    /// Runs the chain may start.
    pub max_runs: u32,
    /// Seconds one run of it may take.
    pub timeout_sec: u64,
}

impl From<&super::Defaults> for Rails {
    fn from(defaults: &super::Defaults) -> Self {
        Self {
            max_hops: defaults.max_hops,
            fuel_usd: defaults.fuel_usd,
            max_runs: defaults.max_runs,
            timeout_sec: defaults.timeout_sec,
        }
    }
}

impl Config {
    /// The rails of a chain of `pipeline` that starts at `starts_at`.
    ///
    /// `None` for either is a chain no pipeline opened, or one whose first agent is not known
    /// here; each then falls back a level.
    #[must_use]
    pub fn rails_for(
        &self,
        pipeline: Option<&PipelineName>,
        starts_at: Option<&AgentName>,
    ) -> Rails {
        let defaults = Rails::from(&self.defaults);
        let declared = pipeline.and_then(|name| self.pipelines.get(name));
        let agent_fuel = starts_at
            .and_then(|agent| self.agents.get(agent))
            .and_then(|agent| agent.fuel_usd);

        Rails {
            max_hops: declared
                .and_then(|p| p.max_hops)
                .unwrap_or(defaults.max_hops),
            fuel_usd: declared
                .and_then(|p| p.fuel_usd)
                .or(agent_fuel)
                .unwrap_or(defaults.fuel_usd),
            max_runs: declared
                .and_then(|p| p.max_runs)
                .unwrap_or(defaults.max_runs),
            timeout_sec: declared
                .and_then(|p| p.timeout_sec)
                .unwrap_or(defaults.timeout_sec),
        }
    }

    /// The rails a chain of `pipeline` starts with when its way in fires: from its entry agent.
    #[must_use]
    pub fn rails_of_pipeline(&self, pipeline: &PipelineName) -> Rails {
        let entry = self
            .pipelines
            .get(pipeline)
            .and_then(|declared| declared.entry.as_ref());
        self.rails_for(Some(pipeline), entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACTORY: &str = r#"
[defaults]
max_hops = 8
fuel_usd = 5.0
max_runs = 64
timeout = "15m"

[agents.analyst]
prompt = "a"

[agents.reviewer]
prompt = "r"
fuel_usd = 2.0

[pipelines.development]
entry = "analyst"

[pipelines.sweep]
entry = "reviewer"
max_hops = 6
fuel_usd = 20.0
max_runs = 12
timeout = "30m"
"#;

    fn config() -> Config {
        Config::from_toml(FACTORY, "rails.toml").expect("parses")
    }

    #[test]
    fn a_workflow_that_says_nothing_runs_on_the_factory_defaults() {
        assert_eq!(
            config().rails_of_pipeline(&"development".into()),
            Rails {
                max_hops: 8,
                fuel_usd: 5.0,
                max_runs: 64,
                timeout_sec: 900,
            }
        );
    }

    #[test]
    fn a_workflow_sets_the_rails_of_its_own_chains() {
        assert_eq!(
            config().rails_of_pipeline(&"sweep".into()),
            Rails {
                max_hops: 6,
                fuel_usd: 20.0,
                max_runs: 12,
                timeout_sec: 1_800,
            },
            "the workflow's Fuel wins over the reviewer's own"
        );
    }

    #[test]
    fn an_agents_own_fuel_is_the_fuel_of_a_chain_that_starts_at_it() {
        let config = config();

        assert!((config.rails_for(None, Some(&"reviewer".into())).fuel_usd - 2.0).abs() < 1e-9);
        assert!(
            (config
                .rails_for(Some(&"development".into()), Some(&"reviewer".into()))
                .fuel_usd
                - 2.0)
                .abs()
                < 1e-9,
            "a chain of development spawned at the reviewer"
        );
        assert!((config.rails_for(None, None).fuel_usd - 5.0).abs() < 1e-9);
    }
}
