//! What a node says about itself: the lines inside its box, and the tooltip over it — and what a
//! route says when it is hovered.
//!
//! An agent's box names the model it runs on and the effort it runs it at, then its context tier
//! and whether it is read-only, because those are what somebody reading a route map most often
//! wants and cannot see anywhere else. The model and its effort share a line because they are one
//! choice — how hard *that* model is asked to reason. Runner and description are in the tooltip,
//! because a box wide enough for them would make the diagram twice as wide to say things that are
//! rarely the question.
//!
//! Nothing is cut to make room: what does not fit beside something else moves to the next line,
//! and a box grows a third small line rather than lose a fact. An agent whose command line names
//! no model or effort is drawn exactly as it was before either was shown.

use crate::agent::{Access, Agent, AgentName};
use crate::config::Config;
use crate::diagram::layout::{Edge, EdgeStyle};
use crate::model::ModelChoice;
use crate::pipeline::{Pipeline, PipelineName};

/// Longest line a box holds. Measured in the dashboard's font, thirty characters of a model name is
/// about 130 of the 148 pixels between a box's padding. The tooltip always has the whole of it.
const FITS: usize = 30;

/// The words for one node.
pub(crate) struct Words {
    /// The first small line.
    pub subtitle: Option<String>,
    /// A second small line, when there is more than fits on one.
    pub caption: Option<String>,
    /// A third, for what fits on neither.
    pub footnote: Option<String>,
    /// Everything, for hovering.
    pub tooltip: String,
}

/// The words for an agent, as it runs in `pipeline` — with that workflow's overrides — or as it
/// declares itself when the whole factory is drawn.
pub(crate) fn agent(
    config: &Config,
    name: &AgentName,
    agent: &Agent,
    pipeline: Option<&PipelineName>,
) -> Words {
    let choice = ModelChoice::in_pipeline(config, name, pipeline);
    let effort = choice
        .reasoning_effort
        .as_ref()
        .map(|effort| format!("effort {effort}"));
    let mut rest: Vec<String> = [
        choice.context_label(),
        (agent.access == Access::ReadOnly).then(|| "read-only".to_owned()),
    ]
    .into_iter()
    .flatten()
    .collect();

    let mut lines = Vec::new();
    match (&choice.model, effort) {
        (Some(model), Some(effort))
            if model.chars().count() + 3 + effort.chars().count() <= FITS =>
        {
            lines.push(format!("{model} · {effort}"));
        }
        (model, effort) => {
            lines.extend(model.as_deref().map(fit));
            if let Some(effort) = effort {
                rest.insert(0, effort);
            }
        }
    }
    lines.extend(pack(&rest));
    let mut lines = lines.into_iter();

    let mut tooltip = vec![name.to_string()];
    if let Some(description) = &agent.description {
        tooltip.push(description.clone());
    }
    if let Some(running_on) = choice.summary() {
        tooltip.push(running_on);
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
        subtitle: lines.next(),
        caption: lines.next(),
        footnote: lines.next(),
        tooltip: tooltip.join("\n"),
    }
}

/// Joins facts into as few lines as hold them, in order, starting a new line rather than cutting.
fn pack(facts: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for fact in facts {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 3 + fact.chars().count() <= FITS => {
                line.push_str(" · ");
                line.push_str(fact);
            }
            _ => lines.push(fit(fact)),
        }
    }
    lines
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
        footnote: None,
        tooltip: tooltip.join("\n"),
    }
}

/// What a route says when it is hovered: its two ends, and anything that makes it more than a
/// plain permission.
pub(crate) fn route(from: &str, to: &str, edge: &Edge) -> String {
    let arrow = if edge.both { "⇄" } else { "→" };
    let mut words = vec![format!("{from} {arrow} {to}")];
    match edge.style {
        EdgeStyle::Plain => {}
        EdgeStyle::Entry => words.push("way in".to_owned()),
        EdgeStyle::Joined => words.push(match &edge.label {
            Some(condition) => format!("waits at the barrier ({condition})"),
            None => "waits at the barrier".to_owned(),
        }),
        EdgeStyle::Bypass => words.push("bypasses the barrier".to_owned()),
        EdgeStyle::Spawn => words.push("spawns a new itinerary".to_owned()),
    }
    if !edge.scopes.is_empty() {
        words.push(format!("only in {}", edge.scopes.join(", ")));
    }
    words.join(" · ")
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
