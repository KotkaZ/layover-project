//! How an agent runs in one workflow, where that differs from what the agent declares.
//!
//! An agent is shared by workflows that want different things of it: the same reviewer gates a
//! feature at `xhigh` and sweeps twelve pull requests an hour at `high`. Declaring a second agent
//! for the second workflow would split its memory, its learnings and its place on the map, so the
//! workflow says how its chains run the agent instead.

use serde::Deserialize;

use crate::config::Selection;

/// What a workflow changes about one agent: its model, effort or context, and arguments it adds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentOverride {
    /// The model this workflow's chains run the agent on, instead of its own.
    #[serde(default)]
    pub model: Option<String>,
    /// The reasoning effort, instead of the agent's own or `[defaults]`.
    #[serde(default)]
    pub effort: Option<String>,
    /// The context-window tier, instead of the agent's own or `[defaults]`.
    #[serde(default)]
    pub context: Option<String>,
    /// Arguments added after the agent's own, for this workflow's chains only.
    ///
    /// Only ever added: a workflow can take more away from an agent, and cannot give back what
    /// the agent's runner or the agent itself denies.
    #[serde(default)]
    pub args: Vec<String>,
}

impl AgentOverride {
    /// Applies this to what the agent would otherwise run with. An empty value changes nothing.
    pub fn apply_to(&self, selection: &mut Selection) {
        fn set(value: Option<&String>) -> Option<String> {
            value.filter(|value| !value.trim().is_empty()).cloned()
        }

        if let Some(model) = set(self.model.as_ref()) {
            selection.model = Some(model);
        }
        if let Some(effort) = set(self.effort.as_ref()) {
            selection.effort = Some(effort);
        }
        if let Some(context) = set(self.context.as_ref()) {
            selection.context = Some(context);
        }
        selection.args.extend(self.args.iter().cloned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_override_replaces_what_it_sets_and_adds_its_arguments() {
        let mut selection = Selection {
            model: Some("claude-opus-5.5".to_owned()),
            effort: Some("xhigh".to_owned()),
            context: Some("long_context".to_owned()),
            name: None,
            args: vec!["--deny-tool=shell(gh:*)".to_owned()],
        };
        AgentOverride {
            effort: Some("high".to_owned()),
            context: Some(" ".to_owned()),
            args: vec!["--deny-tool=shell(git:*)".to_owned()],
            ..AgentOverride::default()
        }
        .apply_to(&mut selection);

        assert_eq!(selection.model.as_deref(), Some("claude-opus-5.5"));
        assert_eq!(selection.effort.as_deref(), Some("high"));
        assert_eq!(
            selection.context.as_deref(),
            Some("long_context"),
            "an empty value changes nothing"
        );
        assert_eq!(
            selection.args,
            ["--deny-tool=shell(gh:*)", "--deny-tool=shell(git:*)"]
        );
    }
}
