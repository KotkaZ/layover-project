//! Agent identity and configuration.
//!
//! An agent's *name* is the key it is declared under in `layover.toml`. Everything else here
//! describes what the agent is for, which matters more than it looks: `layover_peers()` hands
//! these descriptions to a running agent so it can decide where to route work. An agent with no
//! description is a name the mesh cannot reason about.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use std::fmt;

/// The name of an agent, as written in `layover.toml`.
///
/// This is the table key — `[agents.analyst]` declares an agent named `analyst` — and it is what
/// routes, joins and flight envelopes refer to.
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

/// Where an agent's standing instructions come from.
///
/// Exactly one of the two forms must be given. Inline prompts are convenient for short
/// instructions; a prompt file is what allows conditional composition — see [`crate::prompt`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptSpec {
    /// The prompt text, written directly in `layover.toml`.
    Inline(String),
    /// A path to a prompt file, resolved relative to the prompt root.
    File(PathBuf),
}

/// A configured agent.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// One line saying what this agent is.
    ///
    /// Handed to peers by `layover_peers()`, so a sending agent can tell who is worth talking to
    /// without the topology being hard-coded into its prompt.
    #[serde(default)]
    pub description: Option<String>,
    /// A longer statement of what the agent is for and when to route work to it.
    #[serde(default)]
    pub purpose: Option<String>,
    /// Runner to invoke; falls back to [`crate::config::Defaults::runner`].
    #[serde(default)]
    pub runner: Option<String>,
    /// Model identifier passed to the runner.
    #[serde(default)]
    pub model: Option<String>,
    /// The agent's standing instructions, written inline.
    ///
    /// Mutually exclusive with [`Agent::prompt_file`]; use [`Agent::prompt_spec`] rather than
    /// reading either field directly.
    #[serde(default)]
    pub prompt: Option<String>,
    /// The agent's standing instructions, read from a file that may compose others.
    #[serde(default)]
    pub prompt_file: Option<PathBuf>,
    /// Whether the agent may write to the shared workspace.
    #[serde(default)]
    pub access: Access,
    /// Whether a human may send flights directly to this agent.
    ///
    /// A named [`crate::pipeline::Pipeline`] also makes its entry agent reachable. This flag is
    /// the lower-level permission, useful for an agent you want to be able to poke by hand
    /// without declaring a trigger for it.
    #[serde(default)]
    pub entry: bool,
    /// Whether the agent is pinned resident rather than transient.
    #[serde(default)]
    pub resident: bool,
    /// Per-agent Fuel override, applied when an itinerary starts at this agent.
    #[serde(default)]
    pub fuel_usd: Option<f64>,
}

impl Agent {
    /// Returns where this agent's prompt comes from.
    ///
    /// # Errors
    ///
    /// Returns [`PromptSpecError`] when neither form is given or when both are.
    pub fn prompt_spec(&self) -> Result<PromptSpec, PromptSpecError> {
        match (self.prompt.as_ref(), self.prompt_file.as_ref()) {
            (Some(text), None) => Ok(PromptSpec::Inline(text.clone())),
            (None, Some(path)) => Ok(PromptSpec::File(path.clone())),
            (Some(_), Some(_)) => Err(PromptSpecError::Both),
            (None, None) => Err(PromptSpecError::Neither),
        }
    }

    /// Returns the one-line description, or a placeholder when none was configured.
    #[must_use]
    pub fn description_or_placeholder(&self) -> &str {
        self.description
            .as_deref()
            .unwrap_or("(no description configured)")
    }
}

/// Why an agent's prompt configuration is unusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PromptSpecError {
    /// Neither `prompt` nor `prompt_file` was given.
    #[error("neither `prompt` nor `prompt_file` is set")]
    Neither,
    /// Both forms were given, so which one applies is undefined.
    #[error("both `prompt` and `prompt_file` are set; exactly one is allowed")]
    Both,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(body: &str) -> Agent {
        toml::from_str(body).expect("agent parses")
    }

    #[test]
    fn a_name_round_trips_through_display() {
        let name = AgentName::new("analyst");

        assert_eq!(name.as_str(), "analyst");
        assert_eq!(name.to_string(), "analyst");
        assert_eq!(AgentName::from("analyst"), name);
    }

    #[test]
    fn an_inline_prompt_is_recognised() {
        let agent = agent(r#"prompt = "do the thing""#);

        assert_eq!(
            agent.prompt_spec(),
            Ok(PromptSpec::Inline("do the thing".to_owned()))
        );
    }

    #[test]
    fn a_prompt_file_is_recognised() {
        let agent = agent(r#"prompt_file = "prompts/tester.md""#);

        assert_eq!(
            agent.prompt_spec(),
            Ok(PromptSpec::File(PathBuf::from("prompts/tester.md")))
        );
    }

    #[test]
    fn giving_both_prompt_forms_is_rejected() {
        let agent = agent(
            r#"
            prompt = "inline"
            prompt_file = "prompts/tester.md"
            "#,
        );

        assert_eq!(agent.prompt_spec(), Err(PromptSpecError::Both));
    }

    #[test]
    fn giving_neither_prompt_form_is_rejected() {
        let agent = agent(r#"description = "does something""#);

        assert_eq!(agent.prompt_spec(), Err(PromptSpecError::Neither));
    }

    #[test]
    fn description_and_purpose_are_optional_but_preserved() {
        let agent = agent(
            r#"
            description = "Turns a request into a work item"
            purpose = "Longer explanation of when to route here."
            prompt = "analyse"
            "#,
        );

        assert_eq!(
            agent.description.as_deref(),
            Some("Turns a request into a work item")
        );
        assert!(agent.purpose.is_some());
        assert_eq!(
            agent.description_or_placeholder(),
            "Turns a request into a work item"
        );
    }

    #[test]
    fn a_missing_description_falls_back_to_a_placeholder() {
        let agent = agent(r#"prompt = "analyse""#);

        assert_eq!(
            agent.description_or_placeholder(),
            "(no description configured)"
        );
    }
}
