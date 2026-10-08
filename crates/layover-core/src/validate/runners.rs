//! Checks about how runners are written: a preset or a whole command, and nothing said twice.
//!
//! A preset supplies an unattended run's plumbing so a runner says only what is particular to it.
//! The way that goes wrong is a runner or agent repeating what the preset already says — usually a
//! runner migrated from a `command` with its old arguments kept — which hands the CLI the same
//! option twice, and for `--model` two different values, one of which wins by position.

use crate::config::{Cli, Config, Runner, fixes_beside};

use super::Diagnostic;

/// The agent key that sets what a preset option carries, for an option an agent should set that
/// way rather than in its `args`.
fn key_for(option: &str) -> Option<&'static str> {
    match option {
        "--model" => Some("model"),
        "--reasoning-effort" => Some("effort"),
        "--context" => Some("context"),
        _ => None,
    }
}

pub(super) fn check_runner_shapes(config: &Config, found: &mut Vec<Diagnostic>) {
    for (name, runner) in &config.runners {
        match (runner.cli, runner.command.is_empty()) {
            (Some(cli), false) => found.push(Diagnostic::error(format!(
                "runner `{name}` sets both `cli = \"{cli}\"` and `command`; give one. `cli` has \
                 Layover supply the invocation and `args` add to it, `command` spells out all of \
                 it"
            ))),
            (None, true) => found.push(Diagnostic::error(format!(
                "runner `{name}` sets neither `cli` nor `command`, so there is nothing to run. Set \
                 `cli = \"copilot\"` or `\"claude\"`, or a `command`"
            ))),
            (None, false) if !runner.args.is_empty() => found.push(Diagnostic::error(format!(
                "runner `{name}` sets `args` beside a `command`. `args` add to what a `cli` preset \
                 supplies; put them in the `command`"
            ))),
            _ => {}
        }

        if let Some(cli) = runner.cli {
            for arg in &runner.args {
                if let Some(option) = cli.already_supplies(arg) {
                    found.push(Diagnostic::warning(repeated(
                        &format!("runner `{name}`"),
                        cli,
                        option,
                        "remove it from the runner's `args`",
                    )));
                }
            }
        }
    }
}

/// An agent's `args` must not repeat what its runner already says, and must land where its CLI
/// reads them.
pub(super) fn check_agent_args(config: &Config, found: &mut Vec<Diagnostic>) {
    for (name, agent) in &config.agents {
        if agent.args.is_empty() {
            continue;
        }
        let Some((runner_name, runner)) = config.runner_of(agent) else {
            continue;
        };
        let who = format!("agent `{name}`");
        args_fit_runner(&who, runner_name, runner, &agent.args, found);
    }
}

/// Reports what is wrong with `args`, added by `who`, on `runner`.
pub(super) fn args_fit_runner(
    who: &str,
    runner_name: &str,
    runner: &Runner,
    args: &[String],
    found: &mut Vec<Diagnostic>,
) {
    if let Some(cli) = runner.cli {
        for arg in args {
            if let Some(option) = cli.already_supplies(arg) {
                let fix = key_for(option).map_or_else(
                    || "remove it from its `args`".to_owned(),
                    |key| format!("set `{key}` instead"),
                );
                found.push(Diagnostic::warning(repeated(
                    &format!("{who}, on runner `{runner_name}`,"),
                    cli,
                    option,
                    &fix,
                )));
            }
        }
        return;
    }

    let line = runner.line_with(args);
    for placeholder in [Runner::MODEL, Runner::EFFORT, Runner::CONTEXT] {
        let carried = runner.takes(placeholder);
        if carried
            && runner.fixes_beside(placeholder).is_none()
            && let Some(fixed) = fixes_beside(&line, placeholder)
        {
            found.push(Diagnostic::warning(format!(
                "{who} fixes `{fixed}` in its `args` for the option runner `{runner_name}` passes \
                 `{placeholder}` to, so the CLI receives both and keeps whichever comes last. Set \
                 the value on the agent instead"
            )));
        }
    }

    let template = runner.template();
    if !template.iter().any(|arg| arg == Runner::ARGS) && template.last().is_some_and(|a| a == "-")
    {
        found.push(Diagnostic::warning(format!(
            "{who} has `args`, and runner `{runner_name}` ends in `-` with no `{}` placeholder, so \
             they would follow the `-` that reads the prompt. Put `{}` before it",
            Runner::ARGS,
            Runner::ARGS
        )));
    }
}

fn repeated(who: &str, cli: Cli, option: &str, fix: &str) -> String {
    format!(
        "{who} repeats `{option}`, which `cli = \"{cli}\"` already supplies, so the CLI would receive it twice; {fix}"
    )
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::validate::{Severity, validate};

    fn factory(runners: &str, agent: &str) -> Config {
        Config::from_toml(
            &format!(
                r#"
[layover]
work_dir = "work"

[defaults]
runner = "main"

{runners}

[agents.analyst]
description = "Analyses"
prompt = "analyse"
model = "claude-opus-5.5"
effort = "high"
context = "default"
entry = true
{agent}
"#
            ),
            "runners.toml",
        )
        .expect("parses")
    }

    fn said(config: &Config, severity: Severity) -> Vec<String> {
        validate(config)
            .into_iter()
            .filter(|d| d.severity == severity)
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn a_preset_runner_with_its_own_denials_and_an_agent_with_its_own_is_clean() {
        let config = factory(
            "[runners.main]\ncli = \"copilot\"\nargs = [\"--deny-tool=shell(git push)\"]",
            "args = [\"--deny-tool=shell(gh:*)\"]",
        );

        assert_eq!(said(&config, Severity::Warning), Vec::<String>::new());
        assert_eq!(said(&config, Severity::Error), Vec::<String>::new());
    }

    #[test]
    fn a_runner_must_be_a_preset_or_a_command_and_not_both() {
        let both = factory(
            "[runners.main]\ncli = \"copilot\"\ncommand = [\"copilot\"]",
            "",
        );
        assert!(
            said(&both, Severity::Error)
                .iter()
                .any(|m| m.contains("sets both"))
        );

        let neither = factory("[runners.main]\nargs = [\"--x\"]", "");
        assert!(
            said(&neither, Severity::Error)
                .iter()
                .any(|m| m.contains("neither `cli` nor `command`"))
        );

        let args_on_command = factory(
            "[runners.main]\ncommand = [\"copilot\", \"--output-format\", \"json\"]\nargs = [\"--x\"]",
            "",
        );
        assert!(
            said(&args_on_command, Severity::Error)
                .iter()
                .any(|m| m.contains("`args` beside a `command`"))
        );
    }

    #[test]
    fn repeating_what_the_preset_supplies_is_a_warning_that_says_what_to_do() {
        let config = factory(
            "[runners.main]\ncli = \"copilot\"\nargs = [\"--no-ask-user\", \"--output-format\", \"json\"]",
            "args = [\"--reasoning-effort=xhigh\"]",
        );
        let warnings = said(&config, Severity::Warning);

        assert!(
            warnings
                .iter()
                .any(|m| m.contains("runner `main` repeats `--no-ask-user`")),
            "{warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|m| m.contains("repeats `--output-format`")),
            "{warnings:?}"
        );
        assert!(
            warnings.iter().any(|m| m.contains("agent `analyst`")
                && m.contains("`--reasoning-effort`")
                && m.contains("set `effort` instead")),
            "{warnings:?}"
        );
    }

    #[test]
    fn an_agent_fixing_a_value_its_command_runner_carries_is_a_warning() {
        let config = factory(
            r#"[runners.main]
command = ["copilot", "--reasoning-effort={effort}", "--output-format", "json"]"#,
            "args = [\"--reasoning-effort\", \"low\"]",
        );

        assert!(
            said(&config, Severity::Warning)
                .iter()
                .any(|m| m.contains("fixes `low`") && m.contains("`{effort}`")),
            "{:?}",
            said(&config, Severity::Warning)
        );
    }

    #[test]
    fn args_after_a_final_dash_are_a_warning() {
        let config = factory(
            r#"[runners.main]
command = ["codex", "exec", "--json", "-"]"#,
            "args = [\"--sandbox\", \"read-only\"]",
        );

        assert!(
            said(&config, Severity::Warning)
                .iter()
                .any(|m| m.contains("would follow the `-`")),
            "{:?}",
            said(&config, Severity::Warning)
        );
    }
}
