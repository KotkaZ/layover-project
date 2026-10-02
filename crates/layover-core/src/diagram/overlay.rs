//! Drawing one chain's journey over its workflow's route map.
//!
//! The route map says what *may* happen. A chain is one thing that *did*: which agents it reached,
//! how often each ran, and which routes the work actually took to get there. Several chains of one
//! workflow at once share one drawing of the workflow, so a map coloured by all of them at once
//! says that the coder is running, and not that it is running in two chains while a third waits for
//! a slot. This is what lets one chain be drawn on its own.
//!
//! Applied after the layout is built, because none of it moves anything: a chain is drawn on the
//! same geometry as its workflow, so the two can be compared at a glance.

use std::collections::BTreeSet;

use crate::agent::AgentName;
use crate::diagram::Live;
use crate::diagram::layout::{Layout, NodeKind, agent_id, pipeline_id};
use crate::pipeline::PipelineName;

/// Where a leg of a chain's journey began.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sender {
    /// The way in: a trigger, a schedule or a resumed layover opened the chain at its entry agent.
    Pipeline(PipelineName),
    /// An agent's flight.
    Agent(AgentName),
}

/// One route a chain's work took: from a sender to the agent it woke.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Leg {
    /// Where the work came from.
    pub from: Sender,
    /// The agent it was for.
    pub to: AgentName,
}

/// How many runs to count on an agent, and over what.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    /// How many.
    pub runs: usize,
    /// Over what, for the box's tooltip: `"in this chain"`, or `"alive now"`.
    pub of: &'static str,
}

/// Marks each node with its tally and each edge with whether the chain took it.
pub(super) fn apply(layout: &mut Layout, live: &Live) {
    for node in &mut layout.nodes {
        if node.kind != NodeKind::Agent {
            continue;
        }
        let Some(tally) = live.tally.get(&AgentName::new(&node.label)) else {
            continue;
        };
        // One run is what a box already implies; a number earns its place only when it says the
        // agent ran again, or runs in more than one place at once.
        if tally.runs > 1 {
            node.badge = Some(format!("×{}", tally.runs));
        }
        let plural = if tally.runs == 1 { "" } else { "s" };
        let note = format!("{} run{plural} {}", tally.runs, tally.of);
        node.tooltip = Some(match node.tooltip.take() {
            Some(tooltip) => format!("{tooltip}\n{note}"),
            None => note,
        });
    }

    let taken: BTreeSet<(String, String)> = live
        .travelled
        .iter()
        .map(|leg| {
            let from = match &leg.from {
                Sender::Pipeline(name) => pipeline_id(name),
                Sender::Agent(name) => agent_id(name),
            };
            (from, agent_id(&leg.to))
        })
        .collect();
    // A line with an arrowhead at each end stands for two routes; either one taken lights it.
    layout.travelled = layout
        .edges
        .iter()
        .filter(|edge| {
            taken.contains(&(edge.from.clone(), edge.to.clone()))
                || (edge.both && taken.contains(&(edge.to.clone(), edge.from.clone())))
        })
        .map(|edge| (edge.from.clone(), edge.to.clone()))
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::diagram::{Activity, Scope, render_svg};

    fn factory() -> Config {
        Config::from_toml(
            r#"
            [layover]
            work_dir = "work"

            [defaults]
            runner = "claude"

            [runners.claude]
            command = ["claude", "-p"]

            [agents.analyst]
            prompt = "analyse"

            [agents.coder]
            prompt = "code"

            [agents.reviewer]
            prompt = "review"

            [agents.helper]
            prompt = "help"

            [pipelines.devforge]
            entry = "analyst"

            [[routes]]
            from = "analyst"
            to = "coder"

            [[routes]]
            from = "coder"
            to = "reviewer"

            [[routes]]
            from = "reviewer"
            to = "coder"

            [[routes]]
            from = "analyst"
            to = "helper"
            "#,
            "overlay-test.toml",
        )
        .expect("config parses")
    }

    /// The chain: the analyst ran, the coder twice around a review, and the reviewer is on its
    /// second look.
    fn chain() -> Live {
        let agent = |name: &str| Sender::Agent(AgentName::new(name));
        Live::default()
            .with("analyst", Activity::Done)
            .with("coder", Activity::Done)
            .with("reviewer", Activity::Running)
            .counted("analyst", 1, "in this chain")
            .counted("coder", 2, "in this chain")
            .counted("reviewer", 2, "in this chain")
            .travelled(Sender::Pipeline(PipelineName::new("devforge")), "analyst")
            .travelled(agent("analyst"), "coder")
            .travelled(agent("coder"), "reviewer")
            .travelled(agent("reviewer"), "coder")
    }

    fn drawn(live: &Live) -> Layout {
        Layout::scoped(
            &factory(),
            live,
            &Scope::Pipeline(PipelineName::new("devforge")),
        )
    }

    /// The opening tag of the route group joining two nodes.
    fn route<'a>(svg: &'a str, from: &str, to: &str) -> &'a str {
        let at = svg
            .find(&format!(r#"data-from="{from}" data-to="{to}""#))
            .or_else(|| svg.find(&format!(r#"data-from="{to}" data-to="{from}""#)))
            .unwrap_or_else(|| panic!("no route {from} - {to} in {svg}"));
        let start = svg[..at].rfind("<g ").expect("a group");
        &svg[start..at]
    }

    #[test]
    fn an_agent_that_ran_more_than_once_carries_a_count_and_says_over_what() {
        let layout = drawn(&chain());

        let coder = layout.node("a_coder").expect("coder");
        assert_eq!(coder.badge.as_deref(), Some("×2"));
        assert!(
            coder
                .tooltip
                .as_deref()
                .is_some_and(|tip| tip.ends_with("\n2 runs in this chain")),
            "{:?}",
            coder.tooltip
        );

        let analyst = layout.node("a_analyst").expect("analyst");
        assert_eq!(analyst.badge, None, "one run is what a box already implies");
        assert!(
            analyst
                .tooltip
                .as_deref()
                .is_some_and(|tip| tip.ends_with("\n1 run in this chain"))
        );
        assert_eq!(layout.node("a_helper").expect("helper").badge, None);
    }

    #[test]
    fn only_the_routes_the_chain_took_are_marked_and_a_two_way_line_lights_either_way() {
        let layout = drawn(&chain());
        let taken = |from: &str, to: &str| {
            layout
                .edges
                .iter()
                .find(|edge| {
                    (edge.from == from && edge.to == to) || (edge.from == to && edge.to == from)
                })
                .unwrap_or_else(|| panic!("no edge {from} - {to}"))
        };
        let taken = |from: &str, to: &str| layout.took(taken(from, to));

        assert!(taken("p_devforge", "a_analyst"), "the trigger opened it");
        assert!(taken("a_analyst", "a_coder"));
        assert!(taken("a_coder", "a_reviewer"), "the review loop, both ways");
        assert!(!taken("a_analyst", "a_helper"), "nobody asked the helper");

        let one_way =
            drawn(&Live::default().travelled(Sender::Agent(AgentName::new("reviewer")), "coder"));
        let pair = one_way
            .edges
            .iter()
            .find(|edge| edge.both)
            .expect("coder and reviewer share a two-way line");
        assert!(
            one_way.took(pair),
            "a send back along a two-way line lights it"
        );
    }

    #[test]
    fn the_drawing_carries_the_count_the_states_and_the_path_taken() {
        let live = chain()
            .with("helper", Activity::Queued)
            .counted("helper", 0, "in this chain");
        let svg = render_svg(&drawn(&live));

        assert!(
            svg.contains(r#"class="node agent done" id="a_analyst""#),
            "{svg}"
        );
        assert!(svg.contains(r#"class="node agent queued" id="a_helper""#));
        assert!(svg.contains(r#"<text class="tally""#), "{svg}");
        assert!(svg.contains(">×2</text>"));
        assert!(route(&svg, "a_analyst", "a_coder").contains("travelled"));
        assert!(!route(&svg, "a_analyst", "a_helper").contains("travelled"));
        assert!(route(&svg, "p_devforge", "a_analyst").contains("travelled"));
    }

    #[test]
    fn a_map_with_no_chain_on_it_marks_nothing() {
        let layout = drawn(&Live::default());

        assert!(layout.nodes.iter().all(|node| node.badge.is_none()));
        assert!(layout.travelled.is_empty());
        assert!(!render_svg(&layout).contains("travelled"));
    }
}
