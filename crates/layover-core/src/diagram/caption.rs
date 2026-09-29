//! What a node says about itself: the lines inside its box, and the tooltip over it.
//!
//! An agent's box names the model it runs on and its context tier, because those are what
//! somebody reading a route map most often wants and cannot see anywhere else. Everything else —
//! reasoning effort, runner, description — is in the tooltip, because a box wide enough for all of
//! it would make the diagram twice as wide to say things that are rarely the question.
//!
//! An agent whose command line names no model is drawn exactly as it was before models were
//! shown, so a factory that sets none sees no change at all.

use crate::agent::{Access, Agent, AgentName};
use crate::config::Config;
use crate::model::ModelChoice;
use crate::pipeline::{Pipeline, PipelineName};

/// Longest text a box holds before it is cut short. The tooltip always has the whole of it.
const FITS: usize = 28;

/// The words for one node.
pub(crate) struct Words {
    /// The first small line.
    pub subtitle: Option<String>,
    /// A second small line, when there is more than fits on one.
    pub caption: Option<String>,
    /// Everything, for hovering.
    pub tooltip: String,
}

/// The words for an agent.
pub(crate) fn agent(config: &Config, name: &AgentName, agent: &Agent) -> Words {
    let choice = ModelChoice::of(config, name);
    let rest: Vec<String> = [
        choice.context_label(),
        (agent.access == Access::ReadOnly).then(|| "read-only".to_owned()),
    ]
    .into_iter()
    .flatten()
    .collect();
    let rest = (!rest.is_empty()).then(|| fit(&rest.join(" · ")));

    let (subtitle, caption) = match &choice.model {
        Some(model) => (Some(fit(model)), rest),
        None => (rest, None),
    };

    let mut tooltip = vec![name.to_string()];
    if let Some(description) = &agent.description {
        tooltip.push(description.clone());
    }
    let running_on: Vec<String> = [
        choice.model.clone(),
        choice
            .reasoning_effort
            .as_ref()
            .map(|effort| format!("reasoning {effort}")),
        choice.context_label(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !running_on.is_empty() {
        tooltip.push(running_on.join(" · "));
    }
    let runner = agent.runner.as_ref().or(config.defaults.runner.as_ref());
    let access = match agent.access {
        Access::ReadOnly => "read-only",
        Access::ReadWrite => "read-write",
    };
    tooltip.push(match runner {
        Some(runner) => format!("runner {runner} · {access}"),
        None => access.to_owned(),
    });

    Words {
        subtitle,
        caption,
        tooltip: tooltip.join("\n"),
    }
}

/// The words for a pipeline.
pub(crate) fn pipeline(name: &PipelineName, pipeline: &Pipeline) -> Words {
    let trigger = pipeline.trigger.to_string();
    let mut tooltip = vec![name.to_string()];
    if let Some(description) = &pipeline.description {
        tooltip.push(description.clone());
    }
    tooltip.push(trigger.clone());

    Words {
        subtitle: Some(trigger),
        caption: None,
        tooltip: tooltip.join("\n"),
    }
}

/// Cuts text that would overflow its box, and says so with an ellipsis.
fn fit(text: &str) -> String {
    if text.chars().count() <= FITS {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(FITS - 1).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn short_text_is_left_alone_and_long_text_is_cut_visibly() {
        assert_eq!(fit("claude-opus-5.5"), "claude-opus-5.5");

        let long = fit("a-model-name-far-too-long-for-any-box");
        assert_eq!(long.chars().count(), super::FITS);
        assert!(long.ends_with('…'));
    }
}
