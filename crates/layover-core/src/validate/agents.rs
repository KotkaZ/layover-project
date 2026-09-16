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
/// The Reserve window must be long enough to hold anything.
///
/// `Reserve::expire` drops entries older than `now - window`. With a window of zero every
/// recorded charge is expired the instant the next authorisation runs, so spend is permanently
/// zero and the cap never refuses anything — an unlimited factory wearing a ceiling.
pub(super) fn check_reserve_window_is_usable(config: &Config, found: &mut Vec<Diagnostic>) {
    if config.reserve.fuel_usd > 0.0 && config.reserve.window_hours == 0 {
        found.push(Diagnostic::error(
            "`[reserve] window_hours` is 0, so every charge expires before the next one is \
             checked and the cap never refuses anything. Set a real window, or set `fuel_usd = 0` \
             if the Reserve is meant to be unlimited"
                .to_owned(),
        ));
    }
}

/// Fuel must be a usable positive number.
///
/// `Itinerary::fuel_exhausted` is `spent >= budget`, so a budget of zero is exhausted before the
/// first run starts: every itinerary is refused and the factory does nothing, with no diagnostic
/// saying why.
///
/// It is worth an explicit check rather than a saturating default because the identically named
/// `[reserve] fuel_usd` documents zero as *unlimited* and implements it that way. The same value
/// in the same-named field means "no ceiling" in one place and "nothing may ever run" in the
/// other, so the one that fails closed has to say so out loud.
pub(super) fn check_fuel_is_usable(config: &Config, found: &mut Vec<Diagnostic>) {
    if config.defaults.fuel_usd <= 0.0 || config.defaults.fuel_usd.is_nan() {
        found.push(Diagnostic::error(format!(
            "`[defaults] fuel_usd` is {}, so every itinerary is out of Fuel before its first run \
             and nothing will ever start. Note that `[reserve] fuel_usd = 0` does mean unlimited, \
             but this one does not",
            config.defaults.fuel_usd
        )));
    }

    for (name, agent) in &config.agents {
        if let Some(fuel) = agent.fuel_usd {
            if fuel <= 0.0 || fuel.is_nan() {
                found.push(Diagnostic::error(format!(
                    "agent `{name}` sets `fuel_usd = {fuel}`, so any itinerary starting there is \
                     out of Fuel before its first run"
                )));
            }
        }
    }
}

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
