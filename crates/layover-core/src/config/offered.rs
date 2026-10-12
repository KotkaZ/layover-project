//! Which flags a workflow offers whoever triggers it.
//!
//! Every workflow accepts every flag `[flags]` declares, because a prompt it reaches might test
//! any of them and a run must never be composed without a value it asks for. But a trigger window
//! listing ten switches, eight of which change nothing in that workflow, teaches people to ignore
//! the switches. So what is *offered* is narrower than what is *accepted*: a workflow's own flags,
//! and the shared ones some prompt its agents read actually tests.

use std::collections::{BTreeMap, BTreeSet};

use super::Config;
use crate::agent::PromptSpec;
use crate::graph::RouteGraph;
use crate::pipeline::{FlagSpec, PipelineName};
use crate::prompt::{PromptSource, referenced_flags};

impl Config {
    /// The flags worth offering whoever triggers `name`: every one it declares itself, and every
    /// one it has from `[flags]` that a prompt of an agent its chains reach tests.
    ///
    /// A prompt that cannot be read offers every shared flag, rather than hide one that matters.
    #[must_use]
    pub fn flags_offered(
        &self,
        name: &PipelineName,
        source: &dyn PromptSource,
    ) -> BTreeMap<String, FlagSpec> {
        let Some(pipeline) = self.pipelines.get(name) else {
            return BTreeMap::new();
        };
        let own = |flag: &String| !pipeline.inherited_flags.contains(flag);
        let mut offered: BTreeMap<String, FlagSpec> = pipeline
            .flags
            .iter()
            .filter(|(flag, _)| own(flag))
            .map(|(flag, spec)| (flag.clone(), spec.clone()))
            .collect();
        if pipeline.inherited_flags.is_empty() {
            return offered;
        }

        let mut tested = BTreeSet::new();
        let mut unreadable = false;
        for agent in RouteGraph::for_pipeline(self, Some(name)).workflow_of(pipeline) {
            let Some(PromptSpec::File(path)) = self
                .agents
                .get(&agent)
                .and_then(|agent| agent.prompt_spec().ok())
            else {
                continue;
            };
            match referenced_flags(source, &path) {
                Ok(flags) => tested.extend(flags),
                Err(_) => unreadable = true,
            }
        }

        for flag in &pipeline.inherited_flags {
            if (unreadable || tested.contains(flag))
                && let Some(spec) = pipeline.flags.get(flag)
            {
                offered.insert(flag.clone(), spec.clone());
            }
        }
        offered
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::PromptMap;

    const FACTORY: &str = r#"
[flags]
publish_pr = { default = false, description = "Publish the pull request" }
run_e2e    = { default = false }

[agents.builder]
prompt_file = "builder.md"

[agents.monitor]
prompt_file = "monitor.md"

[pipelines.development]
entry = "builder"

[pipelines.development.flags]
dry_run = { default = true }

[pipelines.watch]
entry = "monitor"
"#;

    fn prompts() -> PromptMap {
        PromptMap::new()
            .with("builder.md", "Build it.\n@include(publish_pr) publish.md\n")
            .with("publish.md", "Publish it.\n")
            .with("monitor.md", "Watch upstream.\n")
    }

    #[test]
    fn a_workflow_offers_its_own_flags_and_the_shared_ones_its_prompts_test() {
        let config = Config::from_toml(FACTORY, "offered.toml").expect("parses");

        let development = config.flags_offered(&"development".into(), &prompts());
        assert_eq!(
            development.keys().map(String::as_str).collect::<Vec<_>>(),
            ["dry_run", "publish_pr"],
            "run_e2e is accepted but tested by nothing development reaches"
        );

        let watch = config.flags_offered(&"watch".into(), &prompts());
        assert!(watch.is_empty(), "{watch:?}");

        assert!(
            config.pipelines[&"watch".into()]
                .flags
                .contains_key("publish_pr"),
            "offered or not, every workflow accepts every shared flag"
        );
    }

    #[test]
    fn a_prompt_that_cannot_be_read_hides_no_shared_flag() {
        let config = Config::from_toml(FACTORY, "offered.toml").expect("parses");

        let offered = config.flags_offered(&"watch".into(), &PromptMap::new());

        assert_eq!(
            offered.keys().map(String::as_str).collect::<Vec<_>>(),
            ["publish_pr", "run_e2e"]
        );
    }
}
