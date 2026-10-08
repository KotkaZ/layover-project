//! What a person chose for one chain when they started it, beyond its flags: a name to know it by,
//! and the model, effort or context particular agents should run at in it.
//!
//! # Why it belongs to the chain
//!
//! A name is how a person finds *their* run of a workflow among three others going at once — "the
//! login page one" — so it has to stay with everything that run causes: its hand-offs, a chain it
//! spawns, a follow-up days later, the continuation a reply to a help request starts. A per-agent
//! choice is the same: asking for the reviewer at `max` on one difficult change means every review
//! in that change, not the first one. So it travels exactly as flags do, and like flags it is the
//! Tower's record, never something an agent can set.
//!
//! # What a choice may say
//!
//! A model, an effort, a context tier — values a runner already carries through a placeholder, and
//! nothing else. Never arguments: a trigger that could add arguments could take away what a runner
//! denies, and the dashboard is not where permissions are decided.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::agent::AgentName;
use crate::config::{Config, Runner, Selection};
use crate::graph::RouteGraph;
use crate::pipeline::PipelineName;

/// The longest name a chain may be given.
pub const NAME_LIMIT: usize = 120;

/// The longest model, effort or context value a trigger may choose.
const VALUE_LIMIT: usize = 64;

/// What one agent should run at in one chain, where a person chose differently from the factory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentChoice {
    /// The model to run it on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How hard to ask it to reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Its context-window tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

impl AgentChoice {
    /// Returns `true` when it chooses nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.model.is_none() && self.effort.is_none() && self.context.is_none()
    }
}

/// What a person chose for a chain when they started it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chosen {
    /// What the chain is called: "Login page: retry banner".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Agents to run differently in this chain, keyed by agent.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<AgentName, AgentChoice>,
}

/// Why a choice was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct Refused(pub String);

impl Chosen {
    /// Returns `true` when nothing was chosen.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.agents.is_empty()
    }

    /// Checks what a person asked for when triggering `pipeline` (or a bare agent) and returns it
    /// cleaned up: the name trimmed, every value trimmed, empty choices dropped.
    ///
    /// # Errors
    ///
    /// Refuses a name with control characters or over [`NAME_LIMIT`], a choice for an agent that is
    /// not declared or that no chain of the workflow reaches, a value that is not a plain token,
    /// and a value the agent's runner has no placeholder to carry — which would otherwise be
    /// accepted and silently not happen.
    pub fn checked(
        config: &Config,
        pipeline: Option<&PipelineName>,
        name: Option<&str>,
        agents: &BTreeMap<String, AgentChoice>,
    ) -> Result<Self, Refused> {
        let name = name.map(str::trim).filter(|name| !name.is_empty());
        if let Some(name) = name {
            if name.chars().count() > NAME_LIMIT {
                return Err(Refused(format!(
                    "a name may be at most {NAME_LIMIT} characters"
                )));
            }
            if name.chars().any(char::is_control) {
                return Err(Refused("a name is one line of text".to_owned()));
            }
        }

        let reached = pipeline.and_then(|pipeline_name| {
            let pipeline = config.pipelines.get(pipeline_name)?;
            (!pipeline.resumes).then(|| {
                RouteGraph::for_pipeline(config, Some(pipeline_name)).workflow_from(&pipeline.entry)
            })
        });

        let mut chosen = BTreeMap::new();
        for (agent_name, choice) in agents {
            let agent_name = AgentName::from(agent_name.as_str());
            let Some(agent) = config.agents.get(&agent_name) else {
                return Err(Refused(format!(
                    "`{agent_name}` is not an agent in this factory"
                )));
            };
            if reached
                .as_ref()
                .is_some_and(|reached| !reached.contains(&agent_name))
            {
                return Err(Refused(format!(
                    "`{agent_name}` never runs in this workflow, so there is nothing to choose for it"
                )));
            }
            let Some((runner_name, runner)) = config.runner_of(agent) else {
                return Err(Refused(format!("`{agent_name}` has no runner")));
            };

            let mut clean = AgentChoice::default();
            for (key, value, placeholder, slot) in [
                ("model", &choice.model, Runner::MODEL, &mut clean.model),
                ("effort", &choice.effort, Runner::EFFORT, &mut clean.effort),
                (
                    "context",
                    &choice.context,
                    Runner::CONTEXT,
                    &mut clean.context,
                ),
            ] {
                let Some(value) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) else {
                    continue;
                };
                if value.chars().count() > VALUE_LIMIT || !value.chars().all(is_token_char) {
                    return Err(Refused(format!(
                        "`{value}` is not a {key}: use letters, digits and `._:/-` only"
                    )));
                }
                if !runner.takes(placeholder) {
                    return Err(Refused(format!(
                        "`{agent_name}` runs on runner `{runner_name}`, which has no \
                         `{placeholder}` placeholder, so a {key} chosen for it would not reach \
                         its CLI"
                    )));
                }
                *slot = Some(value.to_owned());
            }
            if !clean.is_empty() {
                chosen.insert(agent_name, clean);
            }
        }

        Ok(Self {
            name: name.map(ToOwned::to_owned),
            agents: chosen,
        })
    }

    /// Applies what was chosen for `agent` on top of what it would otherwise run with.
    pub fn apply_to(&self, agent: &AgentName, selection: &mut Selection) {
        if let Some(choice) = self.agents.get(agent) {
            for (value, slot) in [
                (&choice.model, &mut selection.model),
                (&choice.effort, &mut selection.effort),
                (&choice.context, &mut selection.context),
            ] {
                if let Some(value) = value.as_deref().filter(|v| !v.trim().is_empty()) {
                    *slot = Some(value.to_owned());
                }
            }
        }
        selection.name = self.session_name(agent);
    }

    /// What `agent`'s run in this chain calls its session: the chain's name and the agent, kept to
    /// characters any command line passes through untouched. `None` for a chain nobody named.
    #[must_use]
    pub fn session_name(&self, agent: &AgentName) -> Option<String> {
        let name = self.name.as_deref()?;
        let plain: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || " -_.,:#()/+'".contains(c) {
                    c
                } else {
                    ' '
                }
            })
            .collect();
        let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
        (!plain.is_empty()).then(|| format!("{plain} - {agent}"))
    }
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "._:/-".contains(c)
}

#[cfg(test)]
mod tests;
