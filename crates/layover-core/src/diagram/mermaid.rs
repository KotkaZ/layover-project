//! Rendering a factory as a Mermaid diagram.
//!
//! The route map is a graph, and a graph is far easier to check by looking than by reading TOML.
//! This turns a [`Config`] into Mermaid source: the dashboard renders it in the browser, and the
//! same output can be pasted into a README, where GitHub renders it too.
//!
//! # Why generate the source rather than lay the graph out
//!
//! Layout is the hard part of drawing a graph, and Mermaid already does it. What is left is a
//! pure function from configuration to text, which is the kind of thing that can be tested
//! properly — the alternative, asserting on the pixels a layout library produced, is not.
//!
//! It also means the diagram in the documentation can be *generated*, so it cannot drift away
//! from the configuration it claims to describe. A hand-drawn diagram is wrong eventually.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::agent::{Access, AgentName};
use crate::config::Config;
use crate::diagram::{Activity, Live};
use crate::graph::RouteGraph;
use crate::route::Join;

/// Renders the factory's route map as Mermaid source.
///
/// Pipelines appear as their own nodes feeding their entry agent, because "how does work get in"
/// is the first question anyone asks of a diagram like this and the route map alone cannot
/// answer it.
#[must_use]
pub fn route_map(config: &Config, live: &Live) -> String {
    let graph = RouteGraph::from_config(config);
    let mut out = String::from("flowchart LR\n");

    render_pipelines(&mut out, config);
    render_agents(&mut out, config, &graph, live);
    render_edges(&mut out, config, &graph);
    render_classes(&mut out, config, live);

    out
}

/// Draws the entry points, and the edge from each into the agent it wakes.
fn render_pipelines(out: &mut String, config: &Config) {
    if config.pipelines.is_empty() {
        return;
    }

    let _ = writeln!(out, "  subgraph pipelines [\"Ways in\"]");
    let _ = writeln!(out, "    direction TB");
    for (name, pipeline) in &config.pipelines {
        let _ = writeln!(
            out,
            "    {}[\"{}<br/><small>{}</small>\"]",
            pipeline_id(name),
            escape(name.as_str()),
            escape(&pipeline.trigger.to_string())
        );
    }
    let _ = writeln!(out, "  end");
}

/// Draws one node per agent, labelled with what it is and what it may touch.
fn render_agents(out: &mut String, config: &Config, graph: &RouteGraph, live: &Live) {
    for (name, agent) in &config.agents {
        let shape = if graph.join_for(name).is_some() {
            // A hexagon reads as a gate, which is what a barrier is.
            ("{{", "}}")
        } else {
            ("[", "]")
        };

        let mut label = escape(name.as_str());
        if agent.access == Access::ReadOnly {
            // Worth showing on the diagram: it is the difference between an agent that can
            // damage the shared tree and one that cannot.
            label.push_str("<br/><small>read-only</small>");
        }
        if let Some(activity) = live.activity.get(name) {
            let _ = write!(label, "<br/><small>{}</small>", activity.class());
        }

        let _ = writeln!(out, "  {}{}\"{label}\"{}", agent_id(name), shape.0, shape.1);
    }
}

/// Draws the permitted edges, annotating the ones that park flights at a barrier.
fn render_edges(out: &mut String, config: &Config, graph: &RouteGraph) {
    for (name, pipeline) in &config.pipelines {
        let _ = writeln!(
            out,
            "  {} ==> {}",
            pipeline_id(name),
            agent_id(&pipeline.entry)
        );
    }

    let mut drawn = BTreeSet::new();
    for route in &config.routes {
        for from in &route.from {
            for to in &route.to {
                if !drawn.insert((from.clone(), to.clone())) {
                    continue;
                }

                // A barrier constrains only the upstreams it names. Any other permitted sender
                // bypasses it and wakes the agent directly, leaving parked flights untouched, so
                // labelling such an edge with the join condition would state the opposite of
                // what happens. It gets a dotted arrow instead, which reads as going around.
                match graph.join_for(to) {
                    Some(spec) if spec.upstreams.contains(from) => {
                        let label = match spec.join {
                            Join::All => "all",
                            Join::Any => "any",
                        };
                        let _ =
                            writeln!(out, "  {} -- {label} --> {}", agent_id(from), agent_id(to));
                    }
                    Some(_) => {
                        let _ =
                            writeln!(out, "  {} -. bypasses .-> {}", agent_id(from), agent_id(to));
                    }
                    None => {
                        let _ = writeln!(out, "  {} --> {}", agent_id(from), agent_id(to));
                    }
                }
            }
        }
    }
}

/// Emits the colour classes, and only the ones actually used.
///
/// Mermaid tolerates an unused `classDef`, but emitting the full palette every time would put
/// three lines of noise in a diagram that is mostly meant to be read as text.
fn render_classes(out: &mut String, config: &Config, live: &Live) {
    if !config.pipelines.is_empty() {
        let _ = writeln!(
            out,
            "  classDef pipeline fill:#eef2ff,stroke:#6366f1,stroke-width:1px;"
        );
        let ids = config
            .pipelines
            .keys()
            .map(pipeline_id)
            .collect::<Vec<_>>()
            .join(",");
        let _ = writeln!(out, "  class {ids} pipeline;");
    }

    for activity in [Activity::Running, Activity::Waiting, Activity::Failed] {
        let members: Vec<String> = live
            .activity
            .iter()
            .filter(|(_, state)| **state == activity)
            .map(|(name, _)| agent_id(name))
            .collect();
        if members.is_empty() {
            continue;
        }

        let style = match activity {
            Activity::Running => "fill:#dcfce7,stroke:#16a34a,stroke-width:2px",
            Activity::Waiting => "fill:#fef9c3,stroke:#ca8a04,stroke-width:2px",
            Activity::Failed => "fill:#fee2e2,stroke:#dc2626,stroke-width:2px",
        };
        let _ = writeln!(out, "  classDef {} {style};", activity.class());
        let _ = writeln!(out, "  class {} {};", members.join(","), activity.class());
    }
}

/// A Mermaid-safe node identifier for an agent.
fn agent_id(name: &AgentName) -> String {
    format!("a_{}", sanitise(name.as_str()))
}

/// A Mermaid-safe node identifier for a pipeline.
fn pipeline_id(name: &crate::pipeline::PipelineName) -> String {
    format!("p_{}", sanitise(name.as_str()))
}

/// Reduces a name to characters Mermaid accepts in an identifier.
///
/// Names are already validated to be `[a-z0-9_]`, so this changes nothing today. It exists
/// because a generated diagram that produces a syntax error is far harder to diagnose than one
/// that renders a slightly odd node name, and loosening the name rules later should not be able
/// to break the dashboard.
fn sanitise(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Escapes text going into a quoted Mermaid label.
fn escape(raw: &str) -> String {
    raw.replace('"', "&quot;").replace('<', "&lt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(body: &str) -> Config {
        Config::from_toml(body, "diagram-test.toml").expect("config parses")
    }

    fn factory() -> Config {
        config(
            r#"
            [layover]
            work_dir = "work"

            [defaults]
            runner = "claude"

            [runners.claude]
            command = ["claude", "-p"]

            [agents.analyst]
            prompt = "analyse"

            [agents.investigator]
            prompt = "investigate"
            access = "read-only"

            [agents.developer]
            prompt = "develop"

            [pipelines.triage]
            entry = "analyst"

            [[routes]]
            from = "analyst"
            to = "investigator"

            [[routes]]
            from = ["analyst", "investigator"]
            to = "developer"
            join = "all"
            "#,
        )
    }

    #[test]
    fn every_agent_and_route_reaches_the_diagram() {
        let mermaid = route_map(&factory(), &Live::default());

        assert!(mermaid.starts_with("flowchart LR\n"));
        for agent in ["a_analyst", "a_investigator", "a_developer"] {
            assert!(mermaid.contains(agent), "{agent} missing from\n{mermaid}");
        }
        assert!(mermaid.contains("a_analyst --> a_investigator"));
    }

    #[test]
    fn a_pipeline_is_drawn_as_the_way_in_rather_than_as_an_agent() {
        // "How does work get in" is the first question a diagram like this has to answer, and
        // the route map on its own cannot: an entry agent looks like any other node.
        let mermaid = route_map(&factory(), &Live::default());

        assert!(mermaid.contains(r#"subgraph pipelines ["Ways in"]"#));
        assert!(mermaid.contains("p_triage ==> a_analyst"));
        assert!(mermaid.contains("class p_triage pipeline;"));
    }

    #[test]
    fn a_barrier_is_drawn_as_a_gate_with_its_condition_on_the_arrows() {
        let mermaid = route_map(&factory(), &Live::default());

        assert!(
            mermaid.contains(r#"a_developer{{"developer"}}"#),
            "a joined agent should be a gate, not a box:\n{mermaid}"
        );
        assert!(mermaid.contains("a_analyst -- all --> a_developer"));
        assert!(mermaid.contains("a_investigator -- all --> a_developer"));
    }

    #[test]
    fn read_only_access_is_visible_on_the_node() {
        let mermaid = route_map(&factory(), &Live::default());

        assert!(mermaid.contains("investigator<br/><small>read-only</small>"));
    }

    #[test]
    fn an_idle_factory_emits_no_state_colours() {
        // The diagram has to work before the Tower has ever run anything, and a palette of
        // unused classes is three lines of noise in something meant to be readable as text.
        let mermaid = route_map(&factory(), &Live::default());

        assert!(!mermaid.contains("classDef running"));
        assert!(!mermaid.contains("classDef waiting"));
        assert!(!mermaid.contains("classDef failed"));
    }

    #[test]
    fn live_state_colours_only_the_agents_in_it() {
        let live = Live::default()
            .with("developer", Activity::Running)
            .with("analyst", Activity::Failed);

        let mermaid = route_map(&factory(), &live);

        assert!(mermaid.contains("class a_developer running;"));
        assert!(mermaid.contains("class a_analyst failed;"));
        assert!(
            !mermaid.contains("classDef waiting"),
            "nothing is waiting, so no class for it"
        );
    }

    #[test]
    fn a_sender_the_barrier_does_not_name_is_drawn_as_bypassing_it() {
        // A barrier constrains only the upstreams it names; any other permitted sender wakes the
        // agent directly. Labelling that edge with the join condition would state the opposite of
        // what happens, which is exactly the misreading that makes a joined agent look like it
        // can never also be an entry point. Found by generating the reference factory and
        // looking at it.
        let config = config(
            r#"
            [layover]
            work_dir = "work"

            [defaults]
            runner = "claude"

            [runners.claude]
            command = ["claude", "-p"]

            [agents.scanner]
            prompt = "scan"

            [agents.left]
            prompt = "left"

            [agents.right]
            prompt = "right"

            [agents.collector]
            prompt = "collect"

            [pipelines.go]
            entry = "scanner"

            [[routes]]
            from = ["left", "right"]
            to = "collector"
            join = "all"

            [[routes]]
            from = "scanner"
            to = "collector"
            "#,
        );

        let mermaid = route_map(&config, &Live::default());

        assert!(mermaid.contains("a_left -- all --> a_collector"));
        assert!(mermaid.contains("a_right -- all --> a_collector"));
        assert!(
            mermaid.contains("a_scanner -. bypasses .-> a_collector"),
            "scanner is not one of the barrier's upstreams:\n{mermaid}"
        );
        assert!(!mermaid.contains("a_scanner -- all"));
    }

    #[test]
    fn a_repeated_edge_is_drawn_once() {
        // Two routes may name the same pair — a fan-out and a rendezvous can overlap — and
        // Mermaid would draw two parallel arrows for it.
        let config = config(
            r#"
            [layover]
            work_dir = "work"

            [defaults]
            runner = "claude"

            [runners.claude]
            command = ["claude", "-p"]

            [agents.a]
            prompt = "a"

            [agents.b]
            prompt = "b"

            [pipelines.go]
            entry = "a"

            [[routes]]
            from = "a"
            to = "b"

            [[routes]]
            from = "a"
            to = "b"
            "#,
        );

        let mermaid = route_map(&config, &Live::default());

        assert_eq!(mermaid.matches("a_a --> a_b").count(), 1);
    }

    #[test]
    fn a_factory_with_no_pipelines_still_renders() {
        let config = config(
            r#"
            [layover]
            work_dir = "work"

            [defaults]
            runner = "claude"

            [runners.claude]
            command = ["claude", "-p"]

            [agents.lonely]
            prompt = "think"
            entry = true
            "#,
        );

        let mermaid = route_map(&config, &Live::default());

        assert!(mermaid.contains("a_lonely"));
        assert!(!mermaid.contains("subgraph"));
        assert!(!mermaid.contains("classDef pipeline"));
    }

    #[test]
    fn the_trigger_is_shown_under_the_pipeline_name() {
        let config = config(
            r#"
            [layover]
            work_dir = "work"

            [defaults]
            runner = "claude"

            [runners.claude]
            command = ["claude", "-p"]

            [agents.sweeper]
            prompt = "sweep"

            [pipelines.nightly]
            entry = "sweeper"
            trigger = { cron = "0 3 * * *" }
            "#,
        );

        let mermaid = route_map(&config, &Live::default());

        assert!(mermaid.contains("nightly<br/><small>cron"), "{mermaid}");
    }
}
