//! Which model an agent runs on, and with how much reasoning and context.
//!
//! Read from the command line Layover will actually run, not from the agent's declarations alone.
//! A factory is free to fix the model, effort or context in its runner command, and anything that
//! consulted only `agent.model` showed nothing for exactly those factories. The command line is
//! also the only honest answer: a value an agent declares whose runner has no placeholder for it
//! never reaches the CLI, so it is not what the agent runs on. The agent's own `model`, `effort` and
//! `context` are substituted first, so two agents sharing one runner each report their own.
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
    /// What `agent` runs on, read from its runner's command with its own model, effort and context
    /// substituted — so a value the agent declares is reported where its runner carries it, and a
    /// value the runner fixes is reported where it fixes one.
    ///
    /// Empty for an agent that does not exist or whose runner does not, which validation reports
    /// on its own.
    #[must_use]
    pub fn of(config: &Config, agent: &AgentName) -> Self {
        Self::in_pipeline(config, agent, None)
    }

    /// What `agent` runs on in a chain of `pipeline`, with that workflow's overrides applied.
    #[must_use]
    pub fn in_pipeline(
        config: &Config,
        agent: &AgentName,
        pipeline: Option<&crate::pipeline::PipelineName>,
    ) -> Self {
        Self::in_chain(config, agent, pipeline, None)
    }

    /// What `agent` runs on in one chain: its workflow's overrides, and then whatever the person
    /// who triggered the chain chose for it.
    #[must_use]
    pub fn in_chain(
        config: &Config,
        agent: &AgentName,
        pipeline: Option<&crate::pipeline::PipelineName>,
        chosen: Option<&crate::chosen::Chosen>,
    ) -> Self {
        let Some(definition) = config.agents.get(agent) else {
            return Self::default();
        };
        let Some((_, runner)) = config.runner_of(definition) else {
            return Self::default();
        };

        Self::running(runner, config.selection_in(agent, pipeline, chosen))
    }

    /// What `runner`'s command line selects with `selection` filled in.
    #[must_use]
    pub fn running(runner: &crate::config::Runner, selection: crate::config::Selection) -> Self {
        let mut choice = Self::read(&runner.invocation(None, &selection));
        // A placeholder that stands alone, or sits in a flag this does not know — Codex's
        // `-c model_reasoning_effort={effort}` — still carries the value to the CLI.
        if choice.model.is_none() && runner.takes_model() {
            choice.model = selection.model;
        }
        if choice.reasoning_effort.is_none() && runner.takes_effort() {
            choice.reasoning_effort = selection.effort;
        }
        if choice.context.is_none() && runner.takes_context() {
            choice.context = selection.context;
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

    /// What it all comes to, on one line: `claude-opus-5.5 · effort xhigh · long context`, or
    /// `None` when the command line says none of it.
    #[must_use]
    pub fn summary(&self) -> Option<String> {
        let said: Vec<String> = [
            self.model.clone(),
            self.reasoning_effort
                .as_ref()
                .map(|effort| format!("effort {effort}")),
            self.context_label(),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!said.is_empty()).then(|| said.join(" · "))
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

    const SHARED: &str = r#"
[runners.shared]
command = ["copilot", "--model", "{model}", "--reasoning-effort={effort}", "--context", "{context}", "--allow-all-tools"]

[agents.eagle]
prompt = "review"
runner = "shared"
model = "claude-opus-5.5"
effort = "xhigh"
context = "long_context"

[agents.tars]
prompt = "triage"
runner = "shared"
model = "claude-opus-5.5"
effort = "high"
"#;

    fn chosen(model: &str, effort: Option<&str>, context: Option<&str>) -> ModelChoice {
        ModelChoice {
            model: Some(model.to_owned()),
            reasoning_effort: effort.map(ToOwned::to_owned),
            context: context.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn two_agents_on_one_runner_run_at_their_own_effort() {
        // The point of the placeholders: the same permissions, a different effort, and no second
        // runner copying the first's deny list.
        let config = config(SHARED);
        let line = |agent: &str| {
            let definition = &config.agents[&AgentName::from(agent)];
            config.runners["shared"].invocation(None, &config.selection(definition))
        };

        assert_eq!(
            line("eagle"),
            [
                "copilot",
                "--model",
                "claude-opus-5.5",
                "--reasoning-effort=xhigh",
                "--context",
                "long_context",
                "--allow-all-tools"
            ]
        );
        assert_eq!(
            line("tars"),
            [
                "copilot",
                "--model",
                "claude-opus-5.5",
                "--reasoning-effort=high",
                "--allow-all-tools"
            ],
            "no context set, so `--context` is left out with it"
        );

        assert_eq!(
            ModelChoice::of(&config, &"eagle".into()),
            chosen("claude-opus-5.5", Some("xhigh"), Some("long_context"))
        );
        assert_eq!(
            ModelChoice::of(&config, &"tars".into()),
            chosen("claude-opus-5.5", Some("high"), None)
        );
    }

    #[test]
    fn the_defaults_fill_in_what_an_agent_leaves_unset_and_no_more() {
        let config = Config::from_toml(
            r#"
            [layover]
            work_dir = "work"

            [defaults]
            runner = "shared"
            effort = "medium"
            context = "long_context"

            [runners.shared]
            command = ["copilot", "--model", "{model}", "--reasoning-effort", "{effort}", "--context={context}"]

            [agents.plain]
            prompt = "p"
            model = "m"

            [agents.keen]
            prompt = "p"
            model = "m"
            effort = "xhigh"
            context = "default"
            "#,
            "defaults.toml",
        )
        .expect("parses");

        assert_eq!(
            ModelChoice::of(&config, &"plain".into()),
            chosen("m", Some("medium"), Some("long_context"))
        );
        assert_eq!(
            ModelChoice::of(&config, &"keen".into()),
            chosen("m", Some("xhigh"), Some("default")),
            "an agent's own value wins"
        );
    }

    #[test]
    fn an_effort_in_a_flag_this_does_not_read_is_still_reported() {
        // Codex takes effort as a config key. The command carries it; reading it back is not
        // something to guess at, so the declared value is what is reported.
        let config = config(
            r#"
[runners.codex]
command = ["codex", "exec", "--model", "{model}", "-c", "model_reasoning_effort={effort}", "-"]

[agents.a]
prompt = "p"
runner = "codex"
model = "gpt-5.4"
effort = "high"
"#,
        );

        assert_eq!(
            ModelChoice::of(&config, &"a".into()),
            chosen("gpt-5.4", Some("high"), None)
        );
    }

    #[test]
    fn a_runner_that_fixes_its_effort_still_reports_it() {
        // The shape every factory had before the placeholders existed, and which still works: the
        // command says what runs, and an effort the agent declares there is not it.
        let config = config(
            r#"
[runners.analysis]
command = ["copilot", "--model", "claude-opus-5.5", "--reasoning-effort", "xhigh", "--context", "long_context"]

[agents.a]
prompt = "p"
runner = "analysis"
effort = "low"
"#,
        );

        assert_eq!(
            ModelChoice::of(&config, &"a".into()),
            chosen("claude-opus-5.5", Some("xhigh"), Some("long_context"))
        );
    }
}
