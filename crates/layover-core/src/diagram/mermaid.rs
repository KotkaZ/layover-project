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

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::agent::{Access, AgentName};
use crate::config::Config;
use crate::diagram::layout::edge_scopes;
use crate::diagram::{Activity, Live, Scope};
use crate::graph::RouteGraph;
use crate::pipeline::PipelineName;
use crate::route::Join;

/// Renders the factory's route map as Mermaid source.
///
/// Pipelines appear as their own nodes feeding their entry agent, because "how does work get in"
/// is the first question anyone asks of a diagram like this and the route map alone cannot
/// answer it.
#[must_use]
pub fn route_map(config: &Config, live: &Live) -> String {
    route_map_for(config, live, &Scope::Everything)
}

/// Renders one workflow, or the whole factory, as Mermaid source.
///
/// One workflow is drawn over the routes its chains may use, from its own entry, so it contains
/// exactly the agents that workflow can wake. The whole factory draws every route, and labels one
/// that is scoped with the pipelines that may use it; a global route is drawn exactly as it was
/// before routes had scopes.
#[must_use]
pub fn route_map_for(config: &Config, live: &Live, scope: &Scope) -> String {
    let graph = match scope.pipeline() {
        Some(name) => RouteGraph::for_pipeline(config, Some(name)),
        None => RouteGraph::from_config(config),
    };
    let members = scope.pipeline().and_then(|name| {
        config
            .pipelines
            .get(name)
            .map(|pipeline| graph.workflow_from(&pipeline.entry))
    });
    let drawing = Drawing {
        config,
        graph: &graph,
        scope,
        members: members.as_ref(),
    };

    let mut out = String::from("flowchart LR\n");
    drawing.pipelines(&mut out);
    drawing.agents(&mut out, live);
    drawing.edges(&mut out);
    drawing.classes(&mut out, live);
    out
}

/// What is being drawn, so each part of the diagram asks the same question the same way.
struct Drawing<'a> {
    config: &'a Config,
    graph: &'a RouteGraph,
    scope: &'a Scope,
    /// The agents one workflow can wake, or `None` for the whole factory.
    members: Option<&'a BTreeSet<AgentName>>,
}

impl Drawing<'_> {
    fn shows_pipeline(&self, name: &PipelineName) -> bool {
        self.scope.pipeline().is_none_or(|wanted| wanted == name)
    }

    fn shows_agent(&self, name: &AgentName) -> bool {
        self.members.is_none_or(|members| members.contains(name))
    }

    /// Draws the entry points, and the edge from each into the agent it wakes.
    fn pipelines(&self, out: &mut String) {
        if !self
            .config
            .pipelines
            .keys()
            .any(|name| self.shows_pipeline(name))
        {
            return;
        }

        let _ = writeln!(out, "  subgraph pipelines [\"Ways in\"]");
        let _ = writeln!(out, "    direction TB");
        for (name, pipeline) in &self.config.pipelines {
            if !self.shows_pipeline(name) {
                continue;
            }
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
    fn agents(&self, out: &mut String, live: &Live) {
        for (name, agent) in &self.config.agents {
            if !self.shows_agent(name) {
                continue;
            }
            let shape = if self.graph.join_for(name).is_some() {
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
    fn edges(&self, out: &mut String) {
        for (name, pipeline) in &self.config.pipelines {
            if !self.shows_pipeline(name) {
                continue;
            }
            let _ = writeln!(
                out,
                "  {} ==> {}",
                pipeline_id(name),
                agent_id(&pipeline.entry)
            );
        }

        let pipelines_of = match self.scope.pipeline() {
            Some(_) => BTreeMap::new(),
            None => edge_scopes(self.config),
        };

        let mut drawn = BTreeSet::new();
        for route in &self.config.routes {
            if self
                .scope
                .pipeline()
                .is_some_and(|name| !route.applies_to(Some(name)))
            {
                continue;
            }
            for from in &route.from {
                for to in &route.to {
                    if !self.shows_agent(from) || !self.shows_agent(to) {
                        continue;
                    }
                    if !drawn.insert((from.clone(), to.clone())) {
                        continue;
                    }

                    let scoped = pipelines_of
                        .get(&(from.clone(), to.clone()))
                        .filter(|names| !names.is_empty());
                    let _ = writeln!(out, "{}", self.edge(route, from, to, scoped));
                }
            }
        }
    }

    /// One edge, in the syntax that says what kind of edge it is.
    ///
    /// A barrier constrains only the upstreams it names. Any other permitted sender bypasses it
    /// and wakes the agent directly, leaving parked flights untouched, so labelling such an edge
    /// with the join condition would state the opposite of what happens. It gets a dotted arrow
    /// instead, which reads as going around.
    ///
    /// A scoped edge on the whole-factory map carries the pipelines that may use it in its label,
    /// in Mermaid's quoted form because pipeline names may hold hyphens that the bare form reads
    /// as part of an arrow.
    fn edge(
        &self,
        route: &crate::route::Route,
        from: &AgentName,
        to: &AgentName,
        scoped: Option<&Vec<String>>,
    ) -> String {
        let (kind, dotted) = if route.is_spawn() {
            (Some("spawn"), false)
        } else {
            match self.graph.join_for(to) {
                Some(spec) if spec.upstreams.contains(from) => (
                    Some(match spec.join {
                        Join::All => "all",
                        Join::Any => "any",
                    }),
                    false,
                ),
                Some(_) => (Some("bypasses"), true),
                None => (None, false),
            }
        };

        let (a, b) = (agent_id(from), agent_id(to));
        match (scoped, kind, dotted) {
            (Some(names), kind, dotted) => {
                let names = escape(&names.join(", "));
                let label = kind.map_or(names.clone(), |kind| format!("{kind} · {names}"));
                let arrow = if dotted { "-.->" } else { "-->" };
                format!("  {a} {arrow}|\"{label}\"| {b}")
            }
            (None, Some(kind), true) => format!("  {a} -. {kind} .-> {b}"),
            (None, Some(kind), false) => format!("  {a} -- {kind} --> {b}"),
            (None, None, _) => format!("  {a} --> {b}"),
        }
    }

    /// Emits the colour classes, and only the ones actually used.
    ///
    /// Mermaid tolerates an unused `classDef`, but emitting the full palette every time would put
    /// three lines of noise in a diagram that is mostly meant to be read as text.
    fn classes(&self, out: &mut String, live: &Live) {
        let shown: Vec<String> = self
            .config
            .pipelines
            .keys()
            .filter(|name| self.shows_pipeline(name))
            .map(pipeline_id)
            .collect();
        if !shown.is_empty() {
            let _ = writeln!(
                out,
                "  classDef pipeline fill:#eef2ff,stroke:#6366f1,stroke-width:1px;"
            );
            let _ = writeln!(out, "  class {} pipeline;", shown.join(","));
        }

        for activity in [Activity::Running, Activity::Waiting, Activity::Failed] {
            let members: Vec<String> = live
                .activity
                .iter()
                .filter(|(name, state)| **state == activity && self.shows_agent(name))
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
}

/// A Mermaid-safe node identifier for an agent.
fn agent_id(name: &AgentName) -> String {
    format!("a_{}", sanitise(name.as_str()))
}

/// A Mermaid-safe node identifier for a pipeline.
fn pipeline_id(name: &PipelineName) -> String {
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
mod tests;
