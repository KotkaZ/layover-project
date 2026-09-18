//! Checks that every agent's prompt actually composes, for every way it can be reached.
//!
//! These need to read prompt files, so they are separate from the pure-TOML checks and are run
//! through [`crate::validate::validate_prompts`].
//!
//! The subtle part is *which* flags an agent may assume. A run receives the flags of the one
//! pipeline that triggered it, never the union of every pipeline in the factory. So a prompt is
//! only safe if every flag it tests is declared by **each** entry point that can reach it —
//! checking against the union would pass a factory that fails the moment the other pipeline runs.

use std::collections::{BTreeMap, BTreeSet};

use crate::agent::{AgentName, PromptSpec};
use crate::config::Config;
use crate::graph::RouteGraph;
use crate::pipeline::Flags;
use crate::prompt::{PromptSource, referenced_flags, resolve};
use crate::tools::unknown_tools_in;

use super::Diagnostic;

pub(super) fn check_prompt_files(
    config: &Config,
    source: &dyn PromptSource,
    found: &mut Vec<Diagnostic>,
) {
    let referenced = collect_referenced_flags(config, source, found);
    check_flags_are_available_at_every_entry(config, &referenced, found);
    check_tools_exist(config, source, found);
}

/// Refuses a prompt that tells an agent to call a tool Layover does not offer.
///
/// An agent instructed to use a tool it does not have will improvise, and improvising is what a
/// factory is meant not to do unattended. This is also the check that would have caught the drift
/// it was written in response to: eleven tool names were documented across prompts and the book,
/// and none of them existed.
fn check_tools_exist(config: &Config, source: &dyn PromptSource, found: &mut Vec<Diagnostic>) {
    for (name, agent) in &config.agents {
        let text = match agent.prompt_spec() {
            Ok(PromptSpec::Inline(text)) => text,
            Ok(PromptSpec::File(path)) => match resolve(source, &path, &Flags::default()) {
                Ok(text) => text,
                // A prompt that will not compose is already reported elsewhere; saying so twice
                // makes the real problem harder to find.
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        for unknown in unknown_tools_in(&text) {
            found.push(Diagnostic::error(format!(
                "agent `{name}`'s prompt tells it to call `{unknown}`, which is not a tool \
                 Layover offers; an agent told to use a tool it does not have will improvise"
            )));
        }
    }
}

/// Reads every agent's prompt once, reporting anything that does not compose.
///
/// Conditions are not evaluated, so a flag behind a branch that is currently false is still
/// collected: it has to be declared, or the run that turns it on would fail.
fn collect_referenced_flags(
    config: &Config,
    source: &dyn PromptSource,
    found: &mut Vec<Diagnostic>,
) -> BTreeMap<AgentName, BTreeSet<String>> {
    let mut referenced = BTreeMap::new();

    for (name, agent) in &config.agents {
        let Ok(PromptSpec::File(path)) = agent.prompt_spec() else {
            continue;
        };

        match referenced_flags(source, &path) {
            Ok(flags) => {
                referenced.insert(name.clone(), flags);
            }
            Err(error) => found.push(Diagnostic::error(format!(
                "agent `{name}` has an unusable prompt: {error}"
            ))),
        }
    }

    referenced
}

/// Every flag a reachable agent tests must be declared by the entry point that can reach it.
fn check_flags_are_available_at_every_entry(
    config: &Config,
    referenced: &BTreeMap<AgentName, BTreeSet<String>>,
    found: &mut Vec<Diagnostic>,
) {
    if referenced.is_empty() {
        return;
    }

    let graph = RouteGraph::from_config(config);

    for entry in entry_points(config) {
        let reachable = graph.reachable_from([&entry.agent]);

        for (agent, flags) in referenced {
            if !reachable.contains(agent) {
                continue;
            }

            for flag in flags {
                if entry.available.contains(flag.as_str()) {
                    continue;
                }

                found.push(Diagnostic::error(format!(
                    "agent `{agent}` tests flag `{flag}` in its prompt, but {} can reach it \
                     without declaring that flag; the run would fail when the prompt is composed",
                    entry.label
                )));
            }
        }
    }
}

/// One way work can enter the mesh, and the flags a run entering that way would carry.
struct Entry<'a> {
    agent: AgentName,
    available: BTreeSet<&'a str>,
    label: String,
}

fn entry_points(config: &Config) -> Vec<Entry<'_>> {
    let mut entries: Vec<Entry<'_>> = config
        .pipelines
        .iter()
        .map(|(name, pipeline)| Entry {
            agent: pipeline.entry.clone(),
            available: pipeline.flags.keys().map(String::as_str).collect(),
            label: format!("pipeline `{name}`"),
        })
        .collect();

    // A bare `entry = true` agent is triggered without a pipeline, so no flags are declared and
    // none can be supplied. Any conditional prompt downstream of it is unreachable in practice.
    entries.extend(
        config
            .agents
            .iter()
            .filter(|(_, agent)| agent.entry)
            .map(|(name, _)| Entry {
                agent: name.clone(),
                available: BTreeSet::new(),
                label: format!("`entry = true` on agent `{name}`"),
            }),
    );

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::PromptMap;
    use crate::validate::{Severity, validate_prompts};

    fn config(body: &str) -> Config {
        Config::from_toml(
            &format!(
                r#"
                [runners.claude]
                command = ["claude", "-p", "{{prompt}}"]
                {body}
                "#
            ),
            "test.toml",
        )
        .expect("config parses")
    }

    fn errors(config: &Config, source: &PromptMap) -> Vec<String> {
        validate_prompts(config, source)
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.message)
            .collect()
    }

    fn tester_source() -> PromptMap {
        PromptMap::new()
            .with("tester.md", "Run the suite.\n@include(run_e2e) e2e.md\n")
            .with("e2e.md", "Also run the remote suite.\n")
    }

    #[test]
    fn an_inline_prompt_is_not_inspected() {
        let config = config(
            r#"
            [agents.planner]
            runner = "claude"
            prompt = "plan"
            entry = true
            "#,
        );

        assert_eq!(validate_prompts(&config, &PromptMap::new()), Vec::new());
    }

    #[test]
    fn a_missing_prompt_file_is_an_error() {
        let config = config(
            r#"
            [agents.planner]
            runner = "claude"
            prompt_file = "planner.md"
            entry = true
            "#,
        );

        let found = errors(&config, &PromptMap::new());
        assert!(
            found.iter().any(|m| m.contains("does not exist")),
            "got {found:?}"
        );
    }

    #[test]
    fn a_malformed_directive_is_an_error() {
        let config = config(
            r#"
            [agents.tester]
            runner = "claude"
            prompt_file = "tester.md"
            entry = true
            "#,
        );
        let source = PromptMap::new().with("tester.md", "@include(  ) e2e.md\n");

        let found = errors(&config, &source);
        assert!(
            found.iter().any(|m| m.contains("@include directive")),
            "got {found:?}"
        );
    }

    #[test]
    fn a_prompt_testing_a_flag_its_pipeline_declares_is_accepted() {
        let config = config(
            r#"
            [agents.tester]
            runner = "claude"
            prompt_file = "tester.md"

            [pipelines.development]
            entry = "tester"

            [pipelines.development.flags]
            run_e2e = { default = false }
            "#,
        );

        assert_eq!(validate_prompts(&config, &tester_source()), Vec::new());
    }

    #[test]
    fn a_prompt_testing_an_undeclared_flag_is_an_error() {
        let config = config(
            r#"
            [agents.tester]
            runner = "claude"
            prompt_file = "tester.md"

            [pipelines.development]
            entry = "tester"
            "#,
        );

        let found = errors(&config, &tester_source());
        assert!(found.iter().any(|m| m.contains("run_e2e")), "got {found:?}");
    }

    #[test]
    fn a_second_pipeline_that_omits_the_flag_is_an_error() {
        // The check that the union-of-all-flags version got wrong. `nightly` can reach the tester
        // without declaring `run_e2e`, so a nightly run would fail while a development run
        // succeeded — and the factory would look fine at load time.
        let config = config(
            r#"
            [agents.tester]
            runner = "claude"
            prompt_file = "tester.md"

            [pipelines.development]
            entry = "tester"

            [pipelines.development.flags]
            run_e2e = { default = false }

            [pipelines.nightly]
            entry = "tester"
            trigger = { every = "1d" }
            "#,
        );

        let found = errors(&config, &tester_source());
        assert!(
            found
                .iter()
                .any(|m| m.contains("run_e2e") && m.contains("`nightly`")),
            "got {found:?}"
        );
    }

    #[test]
    fn a_flag_must_be_declared_by_every_pipeline_that_reaches_it_transitively() {
        // The tester is two edges downstream, so this is about reachability rather than about
        // being an entry agent.
        let config = config(
            r#"
            [agents.analyst]
            runner = "claude"
            prompt = "analyse"

            [agents.tester]
            runner = "claude"
            prompt_file = "tester.md"

            [[routes]]
            from = "analyst"
            to = "tester"

            [pipelines.development]
            entry = "analyst"

            [pipelines.development.flags]
            run_e2e = { default = false }

            [pipelines.nightly]
            entry = "analyst"
            trigger = { every = "1d" }
            "#,
        );

        let found = errors(&config, &tester_source());
        assert!(
            found.iter().any(|m| m.contains("`nightly`")),
            "a flag must survive every route into the agent, got {found:?}"
        );
    }

    #[test]
    fn a_bare_entry_agent_cannot_reach_a_conditional_prompt() {
        // `entry = true` is triggered without a pipeline, so no flags exist to be supplied. A
        // conditional prompt downstream of it would fail at composition time.
        let config = config(
            r#"
            [agents.tester]
            runner = "claude"
            prompt_file = "tester.md"
            entry = true
            "#,
        );

        let found = errors(&config, &tester_source());
        assert!(
            found.iter().any(|m| m.contains("entry = true")),
            "got {found:?}"
        );
    }

    #[test]
    fn an_agent_no_entry_point_reaches_is_not_flagged_here() {
        // Unreachability is reported by the reach check; reporting it twice, as a prompt error,
        // would be noise.
        let config = config(
            r#"
            [agents.planner]
            runner = "claude"
            prompt = "plan"
            entry = true

            [agents.tester]
            runner = "claude"
            prompt_file = "tester.md"
            "#,
        );

        assert_eq!(errors(&config, &tester_source()), Vec::<String>::new());
    }

    #[test]
    fn a_prompt_that_includes_itself_is_refused_at_load_time() {
        // Validation applies exactly the rules composition does, so a cycle is caught here rather
        // than by the first run that tries to assemble the prompt.
        let config = config(
            r#"
            [agents.tester]
            runner = "claude"
            prompt_file = "tester.md"
            entry = true
            "#,
        );
        let source = PromptMap::new().with("tester.md", "@include tester.md\n");

        let found = errors(&config, &source);
        assert!(
            found.iter().any(|m| m.contains("includes itself")),
            "got {found:?}"
        );
    }

    #[test]
    fn a_prompt_nested_too_deeply_is_refused_at_load_time() {
        let config = config(
            r#"
            [agents.tester]
            runner = "claude"
            prompt_file = "l0.md"
            entry = true
            "#,
        );

        let mut source = PromptMap::new();
        for level in 0..12 {
            source = source.with(
                format!("l{level}.md"),
                format!("@include l{}.md\n", level + 1),
            );
        }
        source = source.with("l12.md", "bottom\n");

        let found = errors(&config, &source);
        assert!(
            found.iter().any(|m| m.contains("nests includes")),
            "got {found:?}"
        );
    }
}
