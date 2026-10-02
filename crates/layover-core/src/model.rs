//! Which model an agent runs on, and with how much reasoning and context.
//!
//! Read from the command line Layover will actually run, not from `agent.model` alone. A factory
//! is free to fix the model in its runner command — one runner per model and permission set is a
//! common shape — and anything that consulted only `agent.model` showed nothing for exactly those
//! factories. The command line is also the only honest answer: a model an agent declares whose
//! runner has no `{model}` placeholder never reaches the CLI, so it is not what the agent runs on.
//!
//! Only flags whose meaning has been confirmed are read: `--model`, which every supported CLI
//! accepts, and Copilot CLI's `--reasoning-effort` and `--context`. A flag this does not know is
//! left alone rather than guessed at, so an agent whose CLI spells these differently shows what
//! it declares and nothing more.

use crate::agent::AgentName;
use crate::config::Config;

/// The model an agent's command line selects, as far as it says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelChoice {
    /// The model identifier, such as `claude-opus-5.5`.
    pub model: Option<String>,
    /// How hard the model is asked to reason, such as `xhigh`.
    pub reasoning_effort: Option<String>,
    /// The context-window tier, such as `long_context`.
    pub context: Option<String>,
}

impl ModelChoice {
    /// What `agent` runs on, read from its runner's command with its own model substituted.
    ///
    /// Empty for an agent that does not exist or whose runner does not, which validation reports
    /// on its own.
    #[must_use]
    pub fn of(config: &Config, agent: &AgentName) -> Self {
        let Some(definition) = config.agents.get(agent) else {
            return Self::default();
        };
        let Some(runner) = definition
            .runner
            .as_ref()
            .or(config.defaults.runner.as_ref())
            .and_then(|name| config.runners.get(name))
        else {
            return Self::default();
        };

        let declared = definition.model.as_deref();
        let mut choice = Self::read(&runner.invocation(None, declared));
        // A placeholder that stands alone, or sits in a flag this does not know, still carries
        // the declared model to the CLI.
        if choice.model.is_none() && runner.takes_model() {
            choice.model = declared.map(ToOwned::to_owned);
        }
        choice
    }

    /// Reads the flags this knows from a command line.
    ///
    /// The last occurrence wins, as it does for the CLIs themselves.
    #[must_use]
    pub fn read(args: &[String]) -> Self {
        Self {
            model: value_of(args, "--model"),
            reasoning_effort: value_of(args, "--reasoning-effort"),
            context: value_of(args, "--context"),
        }
    }

    /// Returns `true` when the command line says none of it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.model.is_none() && self.reasoning_effort.is_none() && self.context.is_none()
    }

    /// The context tier as a person would say it: `long_context` is "long context", and a tier
    /// that does not name itself a context gets the word added.
    #[must_use]
    pub fn context_label(&self) -> Option<String> {
        self.context.as_ref().map(|tier| {
            let words = tier.replace(['_', '-'], " ");
            if words.contains("context") {
                words
            } else {
                format!("{words} context")
            }
        })
    }
}

/// The value a flag is given, as `--flag value` or `--flag=value`.
///
/// A following argument that is itself a flag is not a value: `--model {model}` with no model set
/// leaves a bare `--model` in front of whatever comes next, and reading `--output-format` as a
/// model name would be worse than saying nothing. An unresolved placeholder is not a value either.
pub(crate) fn value_of(args: &[String], flag: &str) -> Option<String> {
    let joined = format!("{flag}=");
    let mut found = None;
    let mut rest = args.iter().peekable();

    while let Some(arg) = rest.next() {
        let value = if arg == flag {
            rest.next_if(|next| !next.starts_with('-')).cloned()
        } else {
            arg.strip_prefix(&joined).map(ToOwned::to_owned)
        };
        if let Some(value) = value.filter(|value| !value.is_empty() && !value.contains('{')) {
            found = Some(value);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn copilots_three_flags_are_read() {
        let choice = ModelChoice::read(&args(&[
            "copilot",
            "--model",
            "claude-opus-5.5",
            "--reasoning-effort",
            "xhigh",
            "--context",
            "long_context",
            "--output-format",
            "json",
        ]));

        assert_eq!(choice.model.as_deref(), Some("claude-opus-5.5"));
        assert_eq!(choice.reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(choice.context.as_deref(), Some("long_context"));
    }

    #[test]
    fn a_joined_flag_is_read_as_readily_as_a_separate_one() {
        let choice = ModelChoice::read(&args(&["codex", "exec", "--model=gpt-5.4", "-"]));

        assert_eq!(choice.model.as_deref(), Some("gpt-5.4"));
    }

    #[test]
    fn the_last_occurrence_wins() {
        let choice = ModelChoice::read(&args(&["copilot", "--model", "a", "--model", "b"]));

        assert_eq!(choice.model.as_deref(), Some("b"));
    }

    #[test]
    fn a_flag_left_without_its_value_says_nothing() {
        // `--model {model}` with no model set leaves `--model` in front of the next flag.
        let choice = ModelChoice::read(&args(&["claude", "-p", "--model", "--output-format"]));

        assert_eq!(choice, ModelChoice::default());
        assert!(choice.is_empty());
    }

    #[test]
    fn an_unresolved_placeholder_is_not_a_model() {
        let choice = ModelChoice::read(&args(&["codex", "exec", "--model={model}", "-"]));

        assert_eq!(choice.model, None);
    }

    #[test]
    fn a_command_that_names_nothing_is_empty() {
        assert!(ModelChoice::read(&args(&["python", "rec.py"])).is_empty());
    }

    #[test]
    fn a_context_tier_reads_as_words() {
        let tier = |context: &str| ModelChoice {
            context: Some(context.to_owned()),
            ..ModelChoice::default()
        };

        assert_eq!(
            tier("long_context").context_label().as_deref(),
            Some("long context")
        );
        assert_eq!(
            tier("default").context_label().as_deref(),
            Some("default context")
        );
        assert_eq!(ModelChoice::default().context_label(), None);
    }

    fn config(body: &str) -> Config {
        Config::from_toml(
            &format!(
                r#"
                [layover]
                work_dir = "work"

                [defaults]
                runner = "templated"

                [runners.templated]
                command = ["claude", "-p", "--model", "{{model}}"]

                [runners.positional]
                command = ["agent", "{{model}}"]

                [runners.fixed]
                command = ["copilot", "--model", "claude-opus-5.5"]

                {body}
                "#
            ),
            "model.toml",
        )
        .expect("parses")
    }

    #[test]
    fn a_declared_model_is_read_through_the_runners_placeholder() {
        let config = config("[agents.a]\nprompt = \"p\"\nmodel = \"claude-sonnet-5\"\n");

        assert_eq!(
            ModelChoice::of(&config, &"a".into()).model.as_deref(),
            Some("claude-sonnet-5")
        );
    }

    #[test]
    fn a_placeholder_that_stands_alone_still_carries_the_declared_model() {
        let config =
            config("[agents.a]\nprompt = \"p\"\nrunner = \"positional\"\nmodel = \"m1\"\n");

        assert_eq!(
            ModelChoice::of(&config, &"a".into()).model.as_deref(),
            Some("m1")
        );
    }

    #[test]
    fn a_model_the_runner_cannot_carry_is_not_the_one_the_agent_runs_on() {
        // Validation warns about the declaration. What the agent runs on is what the command
        // says, and here the command fixes it.
        let config = config("[agents.a]\nprompt = \"p\"\nrunner = \"fixed\"\nmodel = \"m1\"\n");

        assert_eq!(
            ModelChoice::of(&config, &"a".into()).model.as_deref(),
            Some("claude-opus-5.5")
        );
    }

    #[test]
    fn an_agent_with_no_model_anywhere_says_nothing() {
        let config = config("[agents.a]\nprompt = \"p\"\n");

        assert!(ModelChoice::of(&config, &"a".into()).is_empty());
        assert!(ModelChoice::of(&config, &"missing".into()).is_empty());
    }
}
