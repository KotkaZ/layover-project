//! Checks about individual agents: runners, prompts and identity.

use crate::agent::PromptSpecError;
use crate::config::Config;

use super::Diagnostic;

pub(super) fn check_runners_exist(config: &Config, found: &mut Vec<Diagnostic>) {
    for (name, agent) in &config.agents {
        let runner = agent.runner.as_ref().or(config.defaults.runner.as_ref());

        match runner {
            None => found.push(Diagnostic::error(format!(
                "agent `{name}` has no runner and no default runner is set"
            ))),
            Some(runner) if !config.runners.contains_key(runner) => {
                found.push(Diagnostic::error(format!(
                    "agent `{name}` uses unknown runner `{runner}`"
                )));
            }
            Some(_) => {}
        }
    }
}

pub(super) fn check_prompts_are_unambiguous(config: &Config, found: &mut Vec<Diagnostic>) {
    for (name, agent) in &config.agents {
        match agent.prompt_spec() {
            Ok(_) => {}
            Err(PromptSpecError::Neither) => found.push(Diagnostic::error(format!(
                "agent `{name}` sets neither `prompt` nor `prompt_file`"
            ))),
            Err(PromptSpecError::Both) => found.push(Diagnostic::error(format!(
                "agent `{name}` sets both `prompt` and `prompt_file`; exactly one is allowed"
            ))),
        }
    }
}

/// Warns about an agent with no one-line description.
///
/// `layover_peers()` hands descriptions to a running agent so it can choose where to send work.
/// An agent with no description is a bare name, and a peer deciding between `tester` and
/// `reviewer` on names alone is guessing.
pub(super) fn check_agents_are_described(config: &Config, found: &mut Vec<Diagnostic>) {
    for (name, agent) in &config.agents {
        match agent.description.as_deref().map(str::trim) {
            None => found.push(Diagnostic::warning(format!(
                "agent `{name}` has no `description`, so peers discovering it through \
                 `layover_peers()` see only its name"
            ))),
            Some("") => found.push(Diagnostic::warning(format!(
                "agent `{name}` has an empty `description`"
            ))),
            Some(text) if text.contains('\n') => found.push(Diagnostic::warning(format!(
                "agent `{name}` has a multi-line `description`; keep it to one line and put the \
                 detail in `purpose`"
            ))),
            Some(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::validate::testing::{assert_mentions, errors, parse, warnings};

    #[test]
    fn an_unknown_runner_is_an_error() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "nonesuch"
            prompt = "plan"
            entry = true
            "#,
        );

        assert_mentions(&errors(&config), "unknown runner `nonesuch`");
    }

    #[test]
    fn an_agent_with_no_runner_and_no_default_is_an_error() {
        let config = parse(
            r#"
            [agents.planner]
            prompt = "plan"
            entry = true
            "#,
        );

        assert_mentions(&errors(&config), "no runner and no default runner");
    }

    #[test]
    fn an_agent_with_no_prompt_at_all_is_an_error() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            description = "plans"
            entry = true
            "#,
        );

        assert_mentions(&errors(&config), "neither `prompt` nor `prompt_file`");
    }

    #[test]
    fn an_agent_with_both_prompt_forms_is_an_error() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            prompt = "plan"
            prompt_file = "planner.md"
            entry = true
            "#,
        );

        assert_mentions(&errors(&config), "both `prompt` and `prompt_file`");
    }

    #[test]
    fn a_missing_description_warns() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            prompt = "plan"
            entry = true
            "#,
        );

        assert_mentions(&warnings(&config), "has no `description`");
    }

    #[test]
    fn an_empty_description_warns() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            description = "   "
            prompt = "plan"
            entry = true
            "#,
        );

        assert_mentions(&warnings(&config), "empty `description`");
    }

    #[test]
    fn a_multi_line_description_warns() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            description = """
            first line
            second line
            """
            prompt = "plan"
            entry = true
            "#,
        );

        assert_mentions(&warnings(&config), "multi-line `description`");
    }

    #[test]
    fn a_described_agent_is_silent() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            description = "Breaks a goal into tasks"
            purpose = "Route here when a request needs turning into concrete work."
            prompt = "plan"
            entry = true
            "#,
        );

        assert!(warnings(&config).is_empty(), "{:?}", warnings(&config));
    }
}
