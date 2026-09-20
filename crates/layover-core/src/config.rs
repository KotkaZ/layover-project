//! Parsing of the single `layover.toml` factory definition.
//!
//! Unknown fields are rejected rather than ignored: a typo in a factory definition should fail
//! at load time, while a human is still watching, rather than silently changing behaviour.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::agent::{Agent, AgentName};
use crate::cost::{RateCard, Reserve};
use crate::pipeline::{Pipeline, PipelineName};
use crate::route::Route;

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
    /// Directory that `prompt_file` paths are resolved against.
    #[serde(default = "default_prompt_dir")]
    pub prompt_dir: PathBuf,
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
            prompt_dir: default_prompt_dir(),
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
    /// Names of environment variables forwarded to every agent's CLI.
    ///
    /// Most factories run one CLI that needs one credential, and repeating it under every agent
    /// is a list that falls out of step the first time an agent is added. Per-agent
    /// [`crate::Agent::env_from`] adds to this rather than replacing it, so an agent that needs a
    /// token nobody else should hold names only that token.
    #[serde(default)]
    pub env_from: Vec<String>,
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
    /// How many times interrupted work may be restarted automatically.
    ///
    /// A crash loop that restarts itself forever is a fork bomb that looks like resilience, so
    /// recovery is bounded like every other rail. Zero disables automatic recovery.
    #[serde(default = "default_max_recovery_attempts")]
    pub max_recovery_attempts: u32,
    /// How many runs may be live at once, across the whole factory.
    ///
    /// The one rail that protects the machine rather than a budget. Hops bounds depth, Fuel and
    /// the Reserve bound money, the run cap bounds a chain's total — none of them bounds how many
    /// headless CLIs start simultaneously, which is what a fan-out over an unknown number of
    /// items produces. Excess work queues rather than being refused: a busy machine will not be
    /// busy in a minute, so delaying is right where refusing would silently drop work.
    #[serde(default = "default_max_concurrent_runs")]
    pub max_concurrent_runs: usize,
    /// How many generations of spawning separate a chain from the trigger that began it.
    ///
    /// A spawned itinerary gets fresh Hops, so Hops cannot see across chains: without this an
    /// agent that spawns an agent that spawns an agent recurses forever while every individual
    /// chain stays perfectly inside its rails. This is Hops, one level up.
    #[serde(default = "default_max_spawn_generations")]
    pub max_spawn_generations: u32,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            runner: None,
            env_from: Vec::new(),
            max_hops: default_max_hops(),
            fuel_usd: default_fuel_usd(),
            max_runs: default_max_runs(),
            timeout_sec: default_timeout_sec(),
            max_recovery_attempts: default_max_recovery_attempts(),
            max_concurrent_runs: default_max_concurrent_runs(),
            max_spawn_generations: default_max_spawn_generations(),
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
    /// Prepended to the path the flag carries.
    ///
    /// Copilot CLI's `--additional-mcp-config` takes *either* a JSON string or a file path, and
    /// tells the two apart by a leading `@`. Without it the path is read as JSON, which fails as
    /// a parse error about the factory's own configuration rather than anything recognisable.
    ///
    /// Empty for CLIs that take a plain path, which is most of them.
    #[serde(default)]
    pub prefix: String,
}

impl McpWiring {
    /// The argument that follows [`Self::flag`].
    #[must_use]
    pub fn argument(&self, path: &str) -> String {
        format!("{}{path}", self.prefix)
    }
}

/// How to invoke a particular headless agent CLI.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runner {
    /// Command and arguments.
    ///
    /// `{prompt}` substitutes the **path** to the agent's composed instructions, which the Tower
    /// writes into the run's Hangar before spawning. It is not the instructions themselves.
    ///
    /// That distinction is the whole design. A prompt is passed on **stdin**, never on the
    /// command line: Windows caps a command line at 32,767 characters, and real agent prompts go
    /// well past it — a sibling project's review agent composes to roughly 98 KB, three times
    /// over, and its ordinary developer agent to 34 KB. Inlining the prompt would work in every
    /// test written against a small fixture and fail on the first agent worth running.
    ///
    /// So most runners need no placeholder at all. It exists for CLIs that accept a file of
    /// instructions as a flag; those that do not get the instructions prepended to stdin.
    pub command: Vec<String>,
    /// How this runner is told where Layover's MCP server is.
    #[serde(default)]
    pub mcp: Option<McpWiring>,
}

impl Runner {
    /// The placeholder substituted with the path to the composed instructions.
    pub const PROMPT_PATH: &'static str = "{prompt}";

    /// The placeholder substituted with the agent's model.
    ///
    /// Every supported CLI spells its model flag differently — `--model`, `-m`, a config key — so
    /// the spelling stays in the runner command, which is already the one place that knows how to
    /// invoke a given CLI. The alternative, a `model_flag` field, would put half of an invocation
    /// in one place and half in another.
    pub const MODEL: &'static str = "{model}";

    /// The placeholder substituted with this runner's [`McpWiring::flag`] and the path to the
    /// generated MCP configuration — two arguments, not one.
    ///
    /// Optional. A command that does not contain it gets the pair appended at the end, which is
    /// what `claude` and `copilot` want. It exists for commands that end in a positional argument
    /// — `codex exec … -` reads the prompt from stdin and must stay last — where appending would
    /// put a flag after the thing it has to precede.
    pub const MCP: &'static str = "{mcp}";

    /// Returns `true` when this runner wants the instructions as a file it is handed.
    ///
    /// When `false`, the Tower prepends them to the stdin payload instead.
    #[must_use]
    pub fn takes_prompt_path(&self) -> bool {
        self.command
            .iter()
            .any(|arg| arg.contains(Self::PROMPT_PATH))
    }

    /// Returns `true` when this runner can carry an agent's `model`.
    ///
    /// An agent that declares a model whose runner cannot carry it is a silent no-op: the run
    /// happens, on whichever model the CLI defaults to, and nothing says the declaration was
    /// ignored. Validation warns about it rather than letting it pass.
    #[must_use]
    pub fn takes_model(&self) -> bool {
        self.command.iter().any(|arg| arg.contains(Self::MODEL))
    }

    /// Builds the command line for one run.
    ///
    /// Substitution is textual and deliberately so: a placeholder sits inside an argument like
    /// `--model={model}` as readily as it stands alone, and the operator writes whichever their
    /// CLI expects.
    ///
    /// An argument that is *only* a `{model}` placeholder disappears when no model is set, rather
    /// than becoming an empty argument — an empty string in `argv` is not nothing, and several
    /// CLIs treat it as a positional.
    #[must_use]
    pub fn invocation(&self, prompt_path: Option<&str>, model: Option<&str>) -> Vec<String> {
        self.invocation_with_mcp(prompt_path, model, None)
    }

    /// The command to run, with every placeholder resolved.
    ///
    /// `mcp_config` is the path to the file [`McpWiring::format`] describes. When the command
    /// names [`Self::MCP`] the flag and path replace it there; otherwise they are appended, which
    /// is the right answer for every CLI that does not end in a positional argument.
    #[must_use]
    pub fn invocation_with_mcp(
        &self,
        prompt_path: Option<&str>,
        model: Option<&str>,
        mcp_config: Option<&str>,
    ) -> Vec<String> {
        let wiring = self.mcp.as_ref().zip(mcp_config);
        let mut out = Vec::with_capacity(self.command.len() + 2);

        for arg in &self.command {
            if arg == Self::MODEL && model.is_none() {
                continue;
            }

            if arg == Self::MCP {
                if let Some((mcp, path)) = wiring {
                    out.push(mcp.flag.clone());
                    out.push(mcp.argument(path));
                }
                // Dropped when there is nothing to wire: an unresolved placeholder reaching a CLI
                // becomes an argument it does not understand.
                continue;
            }

            let mut rendered = arg.clone();
            if let Some(path) = prompt_path {
                rendered = rendered.replace(Self::PROMPT_PATH, path);
            }
            if let Some(model) = model {
                rendered = rendered.replace(Self::MODEL, model);
            }

            out.push(rendered);
        }

        if let Some((mcp, path)) = wiring
            && !self.command.iter().any(|arg| arg == Self::MCP)
        {
            out.push(mcp.flag.clone());
            out.push(mcp.argument(path));
        }

        out
    }
}

/// A bound on what the whole factory may spend, across every itinerary.
///
/// Fuel bounds one chain. This bounds the factory, which is a different question: a scheduled
/// pipeline mints a fresh itinerary — and a fresh Fuel budget — on every tick, so every chain can
/// stay inside its rail while the total runs away. See [`crate::cost::reserve`].
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReserveConfig {
    /// Ceiling for the window, in US dollars. Zero means unlimited.
    #[serde(default = "default_reserve_usd")]
    pub fuel_usd: f64,
    /// How far back the rolling window reaches.
    ///
    /// Rolling rather than per calendar day, deliberately: a daily bucket can be spent twice
    /// across midnight, and needs a timezone to decide when midnight is.
    #[serde(default = "default_reserve_window_hours")]
    pub window_hours: u64,
}

impl ReserveConfig {
    /// Returns the window as a duration.
    #[must_use]
    pub fn window(&self) -> Duration {
        Duration::from_secs(self.window_hours.saturating_mul(3_600))
    }

    /// Builds the runtime Reserve this configuration describes.
    #[must_use]
    pub fn to_reserve(&self) -> Reserve {
        Reserve::new(self.fuel_usd, self.window())
    }

    /// Returns `true` when no ceiling is enforced.
    #[must_use]
    pub fn is_unlimited(&self) -> bool {
        !(self.fuel_usd.is_finite() && self.fuel_usd > 0.0)
    }
}

impl Default for ReserveConfig {
    fn default() -> Self {
        Self {
            fuel_usd: default_reserve_usd(),
            window_hours: default_reserve_window_hours(),
        }
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
    /// The factory-wide spend ceiling.
    #[serde(default)]
    pub reserve: ReserveConfig,
    /// Published model prices, used only when a runner reports tokens but no cost.
    #[serde(default)]
    pub rates: RateCard,
    /// Available runners, keyed by name.
    #[serde(default)]
    pub runners: BTreeMap<String, Runner>,
    /// Configured agents, keyed by name.
    #[serde(default)]
    pub agents: BTreeMap<AgentName, Agent>,
    /// Named, triggerable entry points, keyed by name.
    #[serde(default)]
    pub pipelines: BTreeMap<PipelineName, Pipeline>,
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

    /// Returns the agents a human or a schedule may send flights to.
    ///
    /// An agent is an entry point when it is marked `entry = true` or when a pipeline names it.
    /// The two are different things: `entry` is a bare permission, while a pipeline is a named
    /// trigger that also carries a schedule and flags.
    ///
    /// Each agent appears once however many pipelines name it.
    pub fn entry_agents(&self) -> impl Iterator<Item = &AgentName> {
        let marked = self
            .agents
            .iter()
            .filter(|(_, agent)| agent.entry)
            .map(|(name, _)| name);

        let piped = self.pipelines.values().map(|pipeline| &pipeline.entry);

        marked.chain(piped).collect::<BTreeSet<_>>().into_iter()
    }

    /// Returns the effective Fuel budget for an itinerary started at `agent`.
    #[must_use]
    pub fn fuel_for(&self, agent: &AgentName) -> f64 {
        self.agents
            .get(agent)
            .and_then(|a| a.fuel_usd)
            .unwrap_or(self.defaults.fuel_usd)
    }

    /// Returns every flag name declared by any pipeline.
    pub fn declared_flags(&self) -> impl Iterator<Item = &str> {
        self.pipelines
            .values()
            .flat_map(|pipeline| pipeline.flags.keys().map(String::as_str))
    }

    /// Returns the pipelines a clock triggers.
    pub fn scheduled_pipelines(&self) -> impl Iterator<Item = (&PipelineName, &Pipeline)> {
        self.pipelines
            .iter()
            .filter(|(_, pipeline)| !pipeline.trigger.is_manual())
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

fn default_state_dir() -> PathBuf {
    PathBuf::from(".layover/state")
}

fn default_work_dir() -> PathBuf {
    PathBuf::from("workspace")
}

fn default_logbook() -> PathBuf {
    PathBuf::from(".layover/logbook.md")
}

fn default_prompt_dir() -> PathBuf {
    PathBuf::from("prompts")
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

/// Four at once: enough for a fan-out to be worth having, few enough that a laptop stays usable.
const fn default_max_concurrent_runs() -> usize {
    4
}

/// One generation. A scanner may spawn a reviewer per item; that reviewer may not spawn more.
const fn default_max_spawn_generations() -> u32 {
    1
}

const fn default_max_runs() -> u32 {
    64
}

const fn default_max_recovery_attempts() -> u32 {
    2
}

const fn default_reserve_usd() -> f64 {
    100.0
}

const fn default_reserve_window_hours() -> u64 {
    24
}

const fn default_timeout_sec() -> u64 {
    900
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::Trigger;

    #[test]
    fn omitted_sections_fall_back_to_defaults() {
        let config = Config::from_toml("", "test.toml").expect("an empty factory parses");

        assert_eq!(config.defaults.max_hops, 8);
        assert_eq!(config.defaults.max_runs, 64);
        assert_eq!(config.layover.http_addr, "127.0.0.1:7878");
        assert_eq!(config.layover.state_dir, PathBuf::from(".layover/state"));
        assert_eq!(config.layover.prompt_dir, PathBuf::from("prompts"));
        assert!(config.pipelines.is_empty());
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
    fn entry_agents_include_marked_agents_and_pipeline_entries() {
        let config = Config::from_toml(
            r#"
            [agents.front_door]
            prompt = "start here"
            entry = true

            [agents.scanner]
            prompt = "poll for work"

            [agents.inner]
            prompt = "not directly reachable"

            [pipelines.review-bot]
            entry = "scanner"
            trigger = { every = "1h" }
            "#,
            "test.toml",
        )
        .expect("config parses");

        let mut entries: Vec<String> = config.entry_agents().map(ToString::to_string).collect();
        entries.sort();

        assert_eq!(entries, ["front_door", "scanner"]);
    }

    #[test]
    fn an_agent_that_is_both_marked_and_piped_is_listed_once() {
        let config = Config::from_toml(
            r#"
            [agents.analyst]
            prompt = "analyse"
            entry = true

            [pipelines.development]
            entry = "analyst"
            "#,
            "test.toml",
        )
        .expect("config parses");

        assert_eq!(config.entry_agents().count(), 1);
    }

    #[test]
    fn two_pipelines_sharing_an_entry_agent_list_it_once() {
        let config = Config::from_toml(
            r#"
            [agents.analyst]
            prompt = "analyse"

            [pipelines.development]
            entry = "analyst"

            [pipelines.nightly]
            entry = "analyst"
            trigger = { every = "1d" }
            "#,
            "test.toml",
        )
        .expect("config parses");

        assert_eq!(
            config.entry_agents().collect::<Vec<_>>(),
            vec![&AgentName::from("analyst")]
        );
    }

    #[test]
    fn scheduled_pipelines_are_separable_from_manual_ones() {
        let config = Config::from_toml(
            r#"
            [agents.analyst]
            prompt = "analyse"

            [agents.scanner]
            prompt = "scan"

            [pipelines.development]
            entry = "analyst"
            trigger = "manual"

            [pipelines.review-bot]
            entry = "scanner"
            trigger = { cron = "0 * * * *" }
            "#,
            "test.toml",
        )
        .expect("config parses");

        let scheduled: Vec<&str> = config
            .scheduled_pipelines()
            .map(|(name, _)| name.as_str())
            .collect();

        assert_eq!(scheduled, ["review-bot"]);
        assert_eq!(
            config.pipelines[&PipelineName::from("development")].trigger,
            Trigger::Manual
        );
    }

    #[test]
    fn declared_flags_are_collected_across_pipelines() {
        let config = Config::from_toml(
            r#"
            [agents.analyst]
            prompt = "analyse"

            [pipelines.development]
            entry = "analyst"

            [pipelines.development.flags]
            run_e2e = { default = false }

            [pipelines.nightly]
            entry = "analyst"
            trigger = { every = "1d" }

            [pipelines.nightly.flags]
            deep_scan = { default = true }
            "#,
            "test.toml",
        )
        .expect("config parses");

        let mut flags: Vec<&str> = config.declared_flags().collect();
        flags.sort_unstable();

        assert_eq!(flags, ["deep_scan", "run_e2e"]);
    }
}

#[cfg(test)]
mod invocation_tests {
    use crate::config::Runner;

    fn runner(args: &[&str]) -> Runner {
        toml::from_str(&format!(
            "command = [{}]",
            args.iter()
                .map(|a| format!("\"{a}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .expect("parses")
    }

    #[test]
    fn a_model_placeholder_is_substituted_wherever_it_sits() {
        // Some CLIs take `--model x`, some take `--model=x`. The operator writes whichever theirs
        // wants, so substitution has to be textual rather than positional.
        let separate = runner(&["claude", "-p", "--model", "{model}"]);
        assert_eq!(
            separate.invocation(None, Some("claude-opus-5")),
            ["claude", "-p", "--model", "claude-opus-5"]
        );

        let joined = runner(&["codex", "exec", "--model={model}"]);
        assert_eq!(
            joined.invocation(None, Some("gpt-5.4")),
            ["codex", "exec", "--model=gpt-5.4"]
        );
    }

    #[test]
    fn a_bare_model_placeholder_disappears_when_no_model_is_set() {
        // An empty string in argv is not nothing; several CLIs read it as a positional argument.
        let r = runner(&["claude", "-p", "{model}"]);
        assert_eq!(r.invocation(None, None), ["claude", "-p"]);
    }

    fn mcp_runner(args: &[&str], flag: &str) -> Runner {
        toml::from_str(&format!(
            "command = [{}]\nmcp = {{ flag = \"{flag}\", format = \"claude_json\" }}",
            args.iter()
                .map(|a| format!("\"{a}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .expect("parses")
    }

    #[test]
    fn mcp_wiring_is_appended_when_the_command_does_not_place_it() {
        // What `claude` and `copilot` want, and what every existing factory file relies on.
        let r = mcp_runner(&["copilot", "--allow-all-tools"], "--mcp-config");
        assert_eq!(
            r.invocation_with_mcp(None, None, Some("/h/mcp.json")),
            [
                "copilot",
                "--allow-all-tools",
                "--mcp-config",
                "/h/mcp.json"
            ]
        );
    }

    #[test]
    fn a_prefix_is_prepended_to_the_path_rather_than_passed_separately() {
        // Copilot CLI's `--additional-mcp-config` takes either a JSON string or a file path and
        // tells them apart by a leading `@`. Passed as its own argument the `@` would be a second
        // value the flag never sees; without it the path is parsed as JSON and the run dies
        // complaining about the factory's own configuration.
        let runner: Runner = toml::from_str(
            r#"command = ["copilot", "--allow-all-tools"]
mcp = { flag = "--additional-mcp-config", format = "claude_json", prefix = "@" }"#,
        )
        .expect("parses");

        assert_eq!(
            runner.invocation_with_mcp(None, None, Some("/h/mcp.json")),
            [
                "copilot",
                "--allow-all-tools",
                "--additional-mcp-config",
                "@/h/mcp.json"
            ]
        );
    }

    #[test]
    fn a_prefix_applies_where_the_command_places_the_wiring_too() {
        // Both branches render the argument, and only one of them having the prefix would be a
        // factory that works until somebody adds `{mcp}` to keep a positional argument last.
        let runner: Runner = toml::from_str(
            r#"command = ["agent", "{mcp}", "-"]
mcp = { flag = "--cfg", format = "claude_json", prefix = "@" }"#,
        )
        .expect("parses");

        assert_eq!(
            runner.invocation_with_mcp(None, None, Some("/h/mcp.json")),
            ["agent", "--cfg", "@/h/mcp.json", "-"]
        );
    }

    #[test]
    fn a_runner_without_a_prefix_still_gets_a_bare_path() {
        let runner = mcp_runner(&["claude", "-p"], "--mcp-config");

        assert_eq!(
            runner.invocation_with_mcp(None, None, Some("/h/mcp.json")),
            ["claude", "-p", "--mcp-config", "/h/mcp.json"]
        );
    }

    #[test]
    fn mcp_wiring_goes_where_the_command_puts_it_when_it_says() {
        // `codex exec … -` reads the prompt from stdin and the `-` has to stay last, so appending
        // would put the flag after the argument it must precede.
        let r = mcp_runner(&["codex", "exec", "{mcp}", "-"], "-c");
        assert_eq!(
            r.invocation_with_mcp(None, None, Some("/h/mcp.toml")),
            ["codex", "exec", "-c", "/h/mcp.toml", "-"]
        );
    }

    #[test]
    fn an_mcp_placeholder_disappears_when_there_is_nothing_to_wire() {
        // An unresolved placeholder reaching a CLI becomes an argument it does not understand.
        let r = mcp_runner(&["codex", "exec", "{mcp}", "-"], "-c");
        assert_eq!(
            r.invocation_with_mcp(None, None, None),
            ["codex", "exec", "-"]
        );
    }

    #[test]
    fn a_runner_with_no_mcp_block_is_wired_to_nothing_even_if_a_path_exists() {
        // The factory always has a config file to offer; only the runner knows whether its CLI
        // can be told about one.
        let r = runner(&["echo", "hello"]);
        assert_eq!(
            r.invocation_with_mcp(None, None, Some("/h/mcp.json")),
            ["echo", "hello"]
        );
    }

    #[test]
    fn the_prompt_path_is_substituted_independently_of_the_model() {
        let r = runner(&["agent", "--file", "{prompt}", "--model", "{model}"]);
        assert_eq!(
            r.invocation(Some("/run/prompt.md"), Some("m1")),
            ["agent", "--file", "/run/prompt.md", "--model", "m1"]
        );
    }

    #[test]
    fn a_runner_without_placeholders_is_passed_through_untouched() {
        let r = runner(&["copilot", "--allow-all-tools"]);
        assert_eq!(
            r.invocation(Some("/x"), Some("m")),
            ["copilot", "--allow-all-tools"]
        );
        assert!(!r.takes_model());
        assert!(!r.takes_prompt_path());
    }

    #[test]
    fn declaring_a_model_a_runner_cannot_carry_is_a_warning() {
        let config: crate::config::Config = toml::from_str(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "claude"

[runners.claude]
command = ["claude", "-p"]

[agents.analyst]
prompt = "analyse"
model = "claude-opus-5"
entry = true
"#,
        )
        .expect("parses");

        let said: Vec<_> = crate::validate::validate(&config)
            .iter()
            .map(|d| d.message.clone())
            .collect();

        assert!(
            said.iter().any(|m| m.contains("no `{model}` placeholder")),
            "{said:?}"
        );
    }
}
