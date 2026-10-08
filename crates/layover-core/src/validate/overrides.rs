//! Checks about `[pipelines.<name>.agents.<agent>]`: that each override names an agent the
//! workflow runs, and that what it changes can reach that agent's CLI.

use crate::config::{Config, Runner};
use crate::graph::RouteGraph;

use super::Diagnostic;

pub(super) fn check_pipeline_overrides(config: &Config, found: &mut Vec<Diagnostic>) {
    for (pipeline_name, pipeline) in &config.pipelines {
        if pipeline.agents.is_empty() {
            continue;
        }
        let reached =
            RouteGraph::for_pipeline(config, Some(pipeline_name)).workflow_from(&pipeline.entry);

        for (agent_name, changes) in &pipeline.agents {
            let who = format!("pipeline `{pipeline_name}`'s override for agent `{agent_name}`");
            let Some(agent) = config.agents.get(agent_name) else {
                found.push(Diagnostic::error(format!(
                    "pipeline `{pipeline_name}` overrides agent `{agent_name}`, which is not \
                     declared"
                )));
                continue;
            };

            // A resuming pipeline's chains wake whichever agent set work down, which its `entry`
            // does not predict, so only an ordinary pipeline's reach is judged.
            if !pipeline.resumes && !reached.contains(agent_name) {
                found.push(Diagnostic::warning(format!(
                    "{who} does nothing: no chain of `{pipeline_name}` can reach `{agent_name}` \
                     over the routes it may use"
                )));
            }

            let Some((runner_name, runner)) = config.runner_of(agent) else {
                continue;
            };
            for (key, value, placeholder) in [
                ("model", changes.model.as_deref(), Runner::MODEL),
                ("effort", changes.effort.as_deref(), Runner::EFFORT),
                ("context", changes.context.as_deref(), Runner::CONTEXT),
            ] {
                match value {
                    Some(value) if value.trim().is_empty() => found.push(Diagnostic::warning(
                        format!("{who} sets `{key} = \"{value}\"`, which changes nothing"),
                    )),
                    Some(value) if !runner.takes(placeholder) => {
                        found.push(Diagnostic::warning(format!(
                            "{who} sets `{key} = \"{value}\"`, but runner `{runner_name}` has no \
                             `{placeholder}` placeholder, so the CLI never sees it"
                        )));
                    }
                    _ => {}
                }
            }

            super::runners::args_fit_runner(&who, runner_name, runner, &changes.args, found);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::validate::{Severity, validate};

    fn factory(overrides: &str) -> Config {
        Config::from_toml(
            &format!(
                r#"
[layover]
work_dir = "work"

[defaults]
runner = "copilot"

[runners.copilot]
cli = "copilot"

[runners.script]
command = ["python", "agent.py"]

[agents.analyst]
description = "Analyses"
prompt = "analyse"
model = "claude-opus-5.5"
effort = "xhigh"
context = "long_context"

[agents.reviewer]
description = "Reviews"
prompt = "review"
model = "claude-opus-5.5"
effort = "xhigh"
context = "long_context"

[agents.bystander]
description = "Not in the sweep"
prompt = "watch"
runner = "script"
entry = true

[pipelines.sweep]
entry = "analyst"

{overrides}

[[routes]]
from = "analyst"
to = "reviewer"
"#
            ),
            "overrides.toml",
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
    fn an_override_for_an_agent_the_workflow_runs_is_clean() {
        let config = factory(
            r#"[pipelines.sweep.agents.reviewer]
effort = "high"
args = ["--deny-tool=shell(git:*)"]"#,
        );

        assert_eq!(said(&config, Severity::Warning), Vec::<String>::new());
        assert_eq!(said(&config, Severity::Error), Vec::<String>::new());
    }

    #[test]
    fn an_override_for_an_undeclared_agent_is_an_error() {
        let config = factory("[pipelines.sweep.agents.ghost]\neffort = \"high\"");

        assert!(
            said(&config, Severity::Error)
                .iter()
                .any(|m| m.contains("overrides agent `ghost`, which is not declared"))
        );
    }

    #[test]
    fn an_override_that_cannot_take_effect_is_a_warning() {
        let config = factory(
            r#"[pipelines.sweep.agents.bystander]
effort = "high""#,
        );
        let warnings = said(&config, Severity::Warning);

        assert!(
            warnings.iter().any(|m| m.contains("does nothing")),
            "{warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|m| m.contains("runner `script` has no `{effort}` placeholder")),
            "{warnings:?}"
        );
    }

    #[test]
    fn an_override_repeating_a_preset_option_says_which_key_to_set() {
        let config = factory(
            r#"[pipelines.sweep.agents.reviewer]
args = ["--context=default"]"#,
        );

        assert!(
            said(&config, Severity::Warning)
                .iter()
                .any(|m| m.contains("override for agent `reviewer`")
                    && m.contains("set `context` instead")),
            "{:?}",
            said(&config, Severity::Warning)
        );
    }
}
