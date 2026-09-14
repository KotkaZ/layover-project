//! Parsing of the single `layover.toml` factory definition.
//!
//! Unknown fields are rejected rather than ignored: a typo in a factory definition should fail
//! at load time, while a human is still watching, rather than silently changing behaviour.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The name of an agent, as written in `layover.toml`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct AgentName(String);

impl AgentName {
    /// Creates an agent name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for AgentName {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Whether an agent may write to the shared workspace.
///
/// Read-only agents are given a worktree snapshot rather than the live tree, which is real
/// enforcement rather than an advisory flag.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    /// Receives a read-only snapshot of the workspace.
    ReadOnly,
    /// Works directly in the shared workspace.
    #[default]
    ReadWrite,
}

/// The condition under which a rendezvous barrier releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Join {
    /// Wait for every declared upstream.
    All,
    /// Release as soon as any one upstream arrives.
    Any,
}

/// Filesystem and network locations used by the Tower.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paths {
    /// Root directory holding one hangar per agent.
    #[serde(default = "default_state_dir")]
    pub state_dir: PathBuf,
    /// The shared working directory agents operate in.
    #[serde(default = "default_work_dir")]
    pub work_dir: PathBuf,
    /// Path to the shared Logbook.
    #[serde(default = "default_logbook")]
    pub logbook: PathBuf,
    /// Address the HTTP API binds to.
    #[serde(default = "default_http_addr")]
    pub http_addr: String,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            state_dir: default_state_dir(),
            work_dir: default_work_dir(),
            logbook: default_logbook(),
            http_addr: default_http_addr(),
        }
    }
}

/// Factory-wide defaults, overridable per agent.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    /// Runner used by agents that do not name one.
    #[serde(default)]
    pub runner: Option<String>,
    /// Maximum depth of a chain, in flights.
    ///
    /// A hop is spent per flight and branches inherit the remaining count, so this bounds depth
    /// and not breadth. See [`crate::itinerary`].
    #[serde(default = "default_max_hops")]
    pub max_hops: u32,
    /// Shared cost budget for an entire itinerary, in US dollars.
    #[serde(default = "default_fuel_usd")]
    pub fuel_usd: f64,
    /// Deterministic backstop on total runs per itinerary.
    ///
    /// Fuel is the intended bound on breadth, but it depends on runners reporting cost. This cap
    /// needs no runner cooperation and is therefore always enforced.
    #[serde(default = "default_max_runs")]
    pub max_runs: u32,
    /// Wall-clock limit for a single run.
    #[serde(default = "default_timeout_sec")]
    pub timeout_sec: u64,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            runner: None,
            max_hops: default_max_hops(),
            fuel_usd: default_fuel_usd(),
            max_runs: default_max_runs(),
            timeout_sec: default_timeout_sec(),
        }
    }
}

/// How Layover passes its MCP endpoint to a runner.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpWiring {
    /// Command-line flag carrying the MCP configuration.
    pub flag: String,
    /// Configuration dialect the runner expects.
    pub format: String,
}

/// How to invoke a particular headless agent CLI.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runner {
    /// Command and arguments; `{prompt}` is substituted at spawn time.
    pub command: Vec<String>,
    /// How this runner is told where Layover's MCP server is.
    #[serde(default)]
    pub mcp: Option<McpWiring>,
}

/// A configured agent.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// Runner to invoke; falls back to [`Defaults::runner`].
    #[serde(default)]
    pub runner: Option<String>,
    /// Model identifier passed to the runner.
    #[serde(default)]
    pub model: Option<String>,
    /// The agent's standing instructions.
    pub prompt: String,
    /// Whether the agent may write to the shared workspace.
    #[serde(default)]
    pub access: Access,
    /// Whether a human may send flights directly to this agent.
    #[serde(default)]
    pub entry: bool,
    /// Whether the agent is pinned resident rather than transient.
    #[serde(default)]
    pub resident: bool,
    /// Per-agent Fuel override.
    #[serde(default)]
    pub fuel_usd: Option<f64>,
}

/// Delivery semantics for an edge.
///
/// v0.1 has a single mode. Blocking request/response was superseded by rendezvous joins, which
/// park flights instead of parking processes. The field exists so that a configuration written
/// against the older design fails with a clear message rather than an unknown-field error.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Fire-and-forget. The sender continues immediately.
    #[default]
    Async,
}

/// A directed edge, or set of edges, in the route map.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    /// Sending agents. Accepts a bare string or a list.
    #[serde(deserialize_with = "one_or_many")]
    pub from: Vec<AgentName>,
    /// Receiving agents. Accepts a bare string or a list.
    #[serde(deserialize_with = "one_or_many")]
    pub to: Vec<AgentName>,
    /// Delivery semantics.
    #[serde(default)]
    pub mode: Mode,
    /// Rendezvous condition. When set, flights are parked until it is met.
    #[serde(default)]
    pub join: Option<Join>,
    /// Backstop for an unreachable barrier.
    #[serde(default)]
    pub timeout_sec: Option<u64>,
}

impl Route {
    /// Returns `true` when this edge parks flights at a barrier.
    #[must_use]
    pub fn is_join(&self) -> bool {
        self.join.is_some()
    }
}

/// A whole factory definition.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Filesystem and network locations.
    #[serde(default)]
    pub layover: Paths,
    /// Factory-wide defaults.
    #[serde(default)]
    pub defaults: Defaults,
    /// Available runners, keyed by name.
    #[serde(default)]
    pub runners: BTreeMap<String, Runner>,
    /// Configured agents, keyed by name.
    #[serde(default)]
    pub agents: BTreeMap<AgentName, Agent>,
    /// The route map.
    #[serde(default)]
    pub routes: Vec<Route>,
}

impl Config {
    /// Parses a factory definition from TOML text.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Parse`] if the text is not a valid factory definition, including
    /// when it carries fields Layover does not recognise.
    pub fn from_toml(text: &str, origin: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|source| ConfigError::Parse {
            path: origin.into(),
            source: Box::new(source),
        })
    }

    /// Reads and parses a factory definition from disk.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Read`] if the file cannot be read, or [`ConfigError::Parse`] if
    /// its contents are not a valid factory definition.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml(&text, path)
    }

    /// Returns the agents a human may send flights to.
    pub fn entry_agents(&self) -> impl Iterator<Item = &AgentName> {
        self.agents
            .iter()
            .filter(|(_, agent)| agent.entry)
            .map(|(name, _)| name)
    }

    /// Returns the effective Fuel budget for an itinerary started at `agent`.
    #[must_use]
    pub fn fuel_for(&self, agent: &AgentName) -> f64 {
        self.agents
            .get(agent)
            .and_then(|a| a.fuel_usd)
            .unwrap_or(self.defaults.fuel_usd)
    }
}

/// Why a factory definition could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("could not read factory definition at {path}")]
    Read {
        /// Path that was attempted.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The file was not a valid factory definition.
    #[error("could not parse factory definition at {path}")]
    Parse {
        /// Path that was attempted.
        path: PathBuf,
        /// Underlying parse failure.
        #[source]
        source: Box<toml::de::Error>,
    },
}

/// Accepts either `from = "a"` or `from = ["a", "b"]`.
fn one_or_many<'de, D>(deserializer: D) -> Result<Vec<AgentName>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        One(AgentName),
        Many(Vec<AgentName>),
    }

    Ok(match Raw::deserialize(deserializer)? {
        Raw::One(name) => vec![name],
        Raw::Many(names) => names,
    })
}

fn default_state_dir() -> PathBuf {
    PathBuf::from(".layover/state")
}

fn default_work_dir() -> PathBuf {
    PathBuf::from("workspace")
}

fn default_logbook() -> PathBuf {
    PathBuf::from(".layover/logbook.md")
}

fn default_http_addr() -> String {
    "127.0.0.1:7878".to_owned()
}

const fn default_max_hops() -> u32 {
    8
}

const fn default_fuel_usd() -> f64 {
    5.0
}

const fn default_max_runs() -> u32 {
    64
}

const fn default_timeout_sec() -> u64 {
    900
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_sections_fall_back_to_defaults() {
        let config = Config::from_toml("", "test.toml").expect("an empty factory parses");

        assert_eq!(config.defaults.max_hops, 8);
        assert_eq!(config.defaults.max_runs, 64);
        assert_eq!(config.layover.http_addr, "127.0.0.1:7878");
        assert_eq!(config.layover.state_dir, PathBuf::from(".layover/state"));
    }

    #[test]
    fn from_and_to_accept_a_string_or_a_list() {
        let config = Config::from_toml(
            r#"
            [[routes]]
            from = "planner"
            to = ["probe_a", "probe_b"]
            "#,
            "test.toml",
        )
        .expect("config parses");

        assert_eq!(config.routes[0].from, vec![AgentName::from("planner")]);
        assert_eq!(config.routes[0].to.len(), 2);
    }

    #[test]
    fn a_typo_is_rejected_rather_than_ignored() {
        let error = Config::from_toml(
            r#"
            [agents.planner]
            prompt = "plan"
            entrypoint = true
            "#,
            "test.toml",
        )
        .expect_err("an unrecognised field must not be silently dropped");

        assert!(matches!(error, ConfigError::Parse { .. }));
    }

    #[test]
    fn access_uses_kebab_case() {
        let config = Config::from_toml(
            r#"
            [agents.reviewer]
            prompt = "review"
            access = "read-only"
            "#,
            "test.toml",
        )
        .expect("config parses");

        assert_eq!(
            config.agents[&AgentName::from("reviewer")].access,
            Access::ReadOnly
        );
    }

    #[test]
    fn the_only_supported_mode_is_async() {
        let config = Config::from_toml(
            r#"
            [[routes]]
            from = "a"
            to = "b"
            mode = "async"
            "#,
            "test.toml",
        )
        .expect("async parses");

        assert_eq!(config.routes[0].mode, Mode::Async);
    }

    #[test]
    fn a_deferred_mode_is_rejected_with_a_clear_error() {
        let error = Config::from_toml(
            r#"
            [[routes]]
            from = "a"
            to = "b"
            mode = "request_response"
            "#,
            "test.toml",
        )
        .expect_err("request_response was superseded by rendezvous joins");

        assert!(matches!(error, ConfigError::Parse { .. }));
    }

    #[test]
    fn a_per_agent_fuel_override_wins() {
        let config = Config::from_toml(
            r#"
            [defaults]
            fuel_usd = 5.0

            [agents.thrifty]
            prompt = "be brief"
            fuel_usd = 1.0

            [agents.spendy]
            prompt = "take your time"
            "#,
            "test.toml",
        )
        .expect("config parses");

        assert!((config.fuel_for(&"thrifty".into()) - 1.0).abs() < 1e-9);
        assert!((config.fuel_for(&"spendy".into()) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn entry_agents_are_listed() {
        let config = Config::from_toml(
            r#"
            [agents.front_door]
            prompt = "start here"
            entry = true

            [agents.inner]
            prompt = "not directly reachable"
            "#,
            "test.toml",
        )
        .expect("config parses");

        let entries: Vec<&AgentName> = config.entry_agents().collect();
        assert_eq!(entries, vec![&AgentName::from("front_door")]);
    }
}
