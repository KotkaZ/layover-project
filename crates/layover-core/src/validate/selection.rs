//! Checks that what an agent asks its CLI for — model, effort, context — actually reaches it.
//!
//! Each is carried by a placeholder in the runner's command. A value with no placeholder to carry
//! it is silently ignored, a placeholder somebody wrote with no value is silently left out, and a
//! runner that fixes a value beside its placeholder hands the CLI two. None of the three stops a
//! run, and all of them mean an agent runs on something other than what `layover.toml` appears to
//! say — the shape of mistake that costs money quietly and is found by accident.

use crate::agent::Agent;
use crate::config::{Config, Defaults, Runner};

use super::Diagnostic;

/// One value an agent can ask for: its key in `layover.toml`, the placeholder that carries it, and
/// where the agent and `[defaults]` keep it.
struct Setting {
    key: &'static str,
    placeholder: &'static str,
    own: fn(&Agent) -> Option<&String>,
    default: fn(&Defaults) -> Option<&String>,
}

const EFFORT: Setting = Setting {
    key: "effort",
    placeholder: Runner::EFFORT,
    own: |agent| agent.effort.as_ref(),
    default: |defaults| defaults.effort.as_ref(),
};

const CONTEXT: Setting = Setting {
    key: "context",
    placeholder: Runner::CONTEXT,
    own: |agent| agent.context.as_ref(),
    default: |defaults| defaults.context.as_ref(),
};

/// An agent's `model` must be able to reach its runner.
///
/// Every supported CLI spells the model flag differently, so it lives in the runner command as a
/// `{model}` placeholder. An agent that declares a model whose runner has no placeholder runs
/// anyway, on whichever model the CLI defaults to, and nothing says the declaration was ignored.
///
/// Unlike effort and context, a `{model}` with no model set is not warned about: factories have
/// relied on a joined `--model={model}` dropping out cleanly, and a new warning would fail their
/// `validate --strict`.
pub(super) fn check_model_reaches_its_runner(config: &Config, found: &mut Vec<Diagnostic>) {
    for (name, agent) in &config.agents {
        let Some(model) = agent.model.as_deref() else {
            continue;
        };
        let Some((runner_name, runner)) = config.runner_of(agent) else {
            continue;
        };

        if !runner.takes_model() {
            found.push(Diagnostic::warning(format!(
                "agent `{name}` sets `model = \"{model}\"`, but runner `{runner_name}` has no `{}` \
                 placeholder, so the run would use the CLI's own default instead",
                Runner::MODEL
            )));
        }
    }
}

/// An agent's `effort` and `context` must reach its runner, and its runner must not be left with
/// nothing to fill them with.
pub(super) fn check_effort_and_context_reach_their_runner(
    config: &Config,
    found: &mut Vec<Diagnostic>,
) {
    for setting in [EFFORT, CONTEXT] {
        let default = (setting.default)(&config.defaults).map(String::as_str);
        if default.is_some_and(|value| value.trim().is_empty()) {
            found.push(Diagnostic::warning(format!(
                "`[defaults] {}` is empty, which counts as unset",
                setting.key
            )));
        }

        for (name, agent) in &config.agents {
            let Some((runner_name, runner)) = config.runner_of(agent) else {
                continue;
            };
            let own = (setting.own)(agent).map(String::as_str);
            let carried = runner.takes(setting.placeholder);

            match own {
                Some(value) if value.trim().is_empty() => {
                    found.push(Diagnostic::warning(format!(
                        "agent `{name}` sets `{} = \"{value}\"`, which counts as unset",
                        setting.key
                    )));
                }
                Some(value) if !carried => found.push(Diagnostic::warning(format!(
                    "agent `{name}` sets `{key} = \"{value}\"`, but runner `{runner_name}` has no \
                     `{placeholder}` placeholder, so the CLI never sees it and the run uses \
                     whatever the runner's command fixes, or the CLI's own default. Add \
                     `{placeholder}` to the runner",
                    key = setting.key,
                    placeholder = setting.placeholder,
                ))),
                // Only a placeholder somebody wrote says they meant it to be filled. A preset's are
                // Layover's, there so an agent *may* set a value, and leaving one unset asks for
                // the CLI's own default.
                None if carried
                    && runner.cli.is_none()
                    && default.is_none_or(|value| value.trim().is_empty()) =>
                {
                    found.push(Diagnostic::warning(format!(
                        "agent `{name}` sets no `{key}`, and `[defaults]` has none, so runner \
                         `{runner_name}` leaves out its `{placeholder}` argument and the option \
                         it belongs to, and the CLI uses its own default. Set `{key}` on the \
                         agent or in `[defaults]`",
                        key = setting.key,
                        placeholder = setting.placeholder,
                    )));
                }
                _ => {}
            }
        }

        check_default_reaches_a_runner(config, &setting, default, found);
    }
}

/// A factory-wide default that no agent relying on it can pass on is a setting that does nothing.
fn check_default_reaches_a_runner(
    config: &Config,
    setting: &Setting,
    default: Option<&str>,
    found: &mut Vec<Diagnostic>,
) {
    let Some(value) = default.filter(|value| !value.trim().is_empty()) else {
        return;
    };

    let mut relying = config
        .agents
        .values()
        .filter(|agent| (setting.own)(agent).is_none())
        .filter_map(|agent| config.runner_of(agent))
        .peekable();

    if relying.peek().is_some() && relying.all(|(_, runner)| !runner.takes(setting.placeholder)) {
        found.push(Diagnostic::warning(format!(
            "`[defaults] {key} = \"{value}\"` reaches no CLI: every agent without a `{key}` of its \
             own runs on a runner with no `{placeholder}` placeholder",
            key = setting.key,
            placeholder = setting.placeholder,
        )));
    }
}

/// A runner that fixes a value beside the placeholder for it hands the CLI two.
///
/// The CLI keeps whichever comes last, so the runner reads as configurable per agent while one
/// order of arguments silently overrides every agent's value, and another silently overrides the
/// runner's.
pub(super) fn check_runners_do_not_fix_what_they_carry(
    config: &Config,
    found: &mut Vec<Diagnostic>,
) {
    // A preset fixes none of these itself; a runner on one that repeats an option is reported as
    // repeating it, which says the same thing more usefully.
    for (name, runner) in config.runners.iter().filter(|(_, r)| r.cli.is_none()) {
        for placeholder in [Runner::MODEL, Runner::EFFORT, Runner::CONTEXT] {
            if let Some(fixed) = runner.fixes_beside(placeholder) {
                found.push(Diagnostic::warning(format!(
                    "runner `{name}` passes `{placeholder}` and also fixes the same option to \
                     `{fixed}`, so the CLI receives both and keeps whichever comes last. Remove \
                     one of them"
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests;
