//! Deciding where each node goes.
//!
//! A layered layout, which is the right shape for a route map: work flows from an entry point
//! towards a terminal agent, and putting each node right of the thing that wakes it makes that
//! flow the diagram's primary axis.
//!
//! # Columns follow reading order, not hop distance
//!
//! Columns used to be breadth-first distance from the way in, the number the hop check computes,
//! so that a diagram could never imply a depth validation disagreed with. That held until a real
//! workflow had shortcuts. The reference factory's analyst may send straight to the reviewer, the
//! developer and the operator for rare messages, which made all three one hop in: they shared a
//! column with the analyst's two consults, the phase order the work follows disappeared, and the
//! tester's one-way sends back became loops under the whole drawing. The map stopped being
//! readable, so a column now says where an agent comes in the work:
//!
//! - `layers` walks depth-first from the way in, each agent's successors in the order
//!   `layover.toml` declares them, sets aside every route that closes a loop, and gives each agent
//!   its longest path over the rest. That terminates on cyclic maps, which longest path alone does
//!   not, and declaration order is what lays a mesh out along its main path.
//! - `stack` gives an edge that skips columns a row of its own in each column it crosses, ordered
//!   with the boxes, so no line passes through a box it does not connect.
//! - `returns` runs each route back just below the columns it spans rather than under the whole
//!   drawing.
//!
//! Hop depth is still worth seeing, so each agent's tooltip says it in words, measured exactly as
//! `layover validate` measures it (`caption::Depths`). The hop check's own numbers are
//! untouched: they never came from here.

use std::collections::{BTreeMap, BTreeSet};

use crate::agent::AgentName;
use crate::config::Config;
use crate::diagram::{Live, Scope, caption};
use crate::graph::RouteGraph;
use crate::pipeline::PipelineName;
use crate::route::Join;

mod layers;
mod model;
mod returns;
mod stack;
mod tidy;

pub use model::{Edge, EdgeStyle, Layout, Node, NodeKind, Shape};

/// Width of a node box.
const NODE_W: f64 = 168.0;
/// Height of a node box.
const NODE_H: f64 = 56.0;
/// Height of a node box when some box in the drawing has a third line to hold.
///
/// One height for the whole drawing rather than per node, so the rows of every column still line
/// up and an edge between two agents in the same row stays straight.
const TALL_NODE_H: f64 = 70.0;
/// Height of a node box when some box in the drawing has a fourth line to hold.
const TALLEST_NODE_H: f64 = 84.0;
/// Horizontal gap between columns.
const COL_GAP: f64 = 96.0;
/// Vertical gap between nodes in a column.
const ROW_GAP: f64 = 32.0;
/// Margin around the whole drawing.
const MARGIN: f64 = 32.0;
/// Vertical spacing between the lanes that return paths are routed through.
const RETURN_GAP: f64 = 34.0;
/// How far left of a node's column the first return path climbs.
const GUTTER_INSET: f64 = 44.0;
/// How much further left each additional return to the same node climbs.
const GUTTER_STEP: f64 = 18.0;
/// Closest to the left edge of the canvas a gutter may be.
const GUTTER_MIN: f64 = 12.0;
/// Closest to a node's right edge that a return path may hook in.
const CORNER_MARGIN: f64 = 20.0;
/// How far right of its column a return path drops.
const EXIT_INSET: f64 = 20.0;
/// How much further right each additional return leaving the same column drops.
const EXIT_STEP: f64 = 12.0;

impl Layout {
    /// Lays out a whole factory.
    #[must_use]
    pub fn build(config: &Config, live: &Live) -> Self {
        Self::scoped(config, live, &Scope::Everything)
    }

    /// Lays out one workflow, or the whole factory.
    ///
    /// One workflow is drawn over the routes its chains may use — global ones and those scoped to
    /// it — so an agent it shares with another workflow appears with only this workflow's edges,
    /// and an agent only another workflow's routes reach does not appear at all.
    #[must_use]
    pub fn scoped(config: &Config, live: &Live, scope: &Scope) -> Self {
        let graph = match scope.pipeline() {
            Some(name) => RouteGraph::for_pipeline(config, Some(name)),
            None => RouteGraph::from_config(config),
        };
        let members = scope.pipeline().and_then(|name| {
            config
                .pipelines
                .get(name)
                .map(|pipeline| graph.workflow_of(pipeline))
        });
        let layers = layers::assign(config, scope, members.as_ref());

        let mut layout = Self::default();
        layout.place(config, live, &graph, &layers, scope, members.as_ref());
        layout.connect(config, &graph, scope, members.as_ref());
        tidy::pair_up(&mut layout.edges);
        layout.stack();
        layout.size();
        super::overlay::apply(&mut layout, live);
        layout
    }

    /// Creates a node for every pipeline and agent, and puts it in its column.
    fn place(
        &mut self,
        config: &Config,
        live: &Live,
        graph: &RouteGraph,
        layers: &BTreeMap<AgentName, usize>,
        scope: &Scope,
        members: Option<&BTreeSet<AgentName>>,
    ) {
        let mut columns: BTreeMap<usize, Vec<Node>> = BTreeMap::new();
        let depths = caption::Depths::of(config, scope);

        for (name, pipeline) in &config.pipelines {
            if scope.pipeline().is_some_and(|wanted| wanted != name) {
                continue;
            }
            let words = caption::pipeline(name, pipeline);
            columns.entry(0).or_default().push(Node {
                id: pipeline_id(name),
                label: name.as_str().to_owned(),
                subtitle: words.subtitle,
                caption: words.caption,
                footnote: words.footnote,
                tooltip: Some(words.tooltip),
                kind: NodeKind::Pipeline,
                shape: Shape::Box,
                activity: None,
                badge: None,
                layer: 0,
                x: 0.0,
                y: 0.0,
                w: NODE_W,
                h: NODE_H,
            });
        }

        for (name, agent) in &config.agents {
            if members.is_some_and(|members| !members.contains(name)) {
                continue;
            }
            // Agents no pipeline can reach still have to appear — an unreachable agent is
            // precisely the thing somebody opened the diagram to find.
            let layer = layers.get(name).copied().unwrap_or(0) + 1;
            let mut words = caption::agent(config, name, agent, scope.pipeline(), &live.chosen);
            if let Some(line) = depths.line(name) {
                words.tooltip.push('\n');
                words.tooltip.push_str(&line);
            }
            columns.entry(layer).or_default().push(Node {
                id: agent_id(name),
                label: name.as_str().to_owned(),
                subtitle: words.subtitle,
                caption: words.caption,
                footnote: words.footnote,
                tooltip: Some(words.tooltip),
                kind: NodeKind::Agent,
                shape: if graph.join_for(name).is_some() {
                    Shape::Gate
                } else {
                    Shape::Box
                },
                activity: live.activity.get(name).copied(),
                badge: None,
                layer,
                x: 0.0,
                y: 0.0,
                w: NODE_W,
                h: NODE_H,
            });
        }

        let nodes = || columns.values().flatten();
        let height = if nodes().any(|node| node.footnote.is_some()) {
            TALLEST_NODE_H
        } else if nodes().any(|node| node.caption.is_some()) {
            TALL_NODE_H
        } else {
            NODE_H
        };

        // Rows are decided by `stack`, once the edges are known.
        for (layer, mut nodes) in columns {
            for node in &mut nodes {
                node.x = column_x(layer);
                node.h = height;
            }
            self.nodes.append(&mut nodes);
        }
    }

    /// Adds the edges, classifying each one.
    fn connect(
        &mut self,
        config: &Config,
        graph: &RouteGraph,
        scope: &Scope,
        members: Option<&BTreeSet<AgentName>>,
    ) {
        let scopes = match scope.pipeline() {
            Some(_) => BTreeMap::new(),
            None => edge_scopes(config),
        };

        for (name, pipeline) in &config.pipelines {
            // A resuming pipeline with no entry wakes whoever set work down: no one agent to point
            // at.
            let Some(entry) = &pipeline.entry else {
                continue;
            };
            if self.node(&pipeline_id(name)).is_none() {
                continue;
            }
            self.edges.push(Edge {
                from: pipeline_id(name),
                to: agent_id(entry),
                label: None,
                style: EdgeStyle::Entry,
                back: false,
                both: false,
                beside: false,
                bulge: None,
                from_y: 0.0,
                to_y: 0.0,
                gutter: None,
                hook_x: None,
                exit_x: None,
                floor: None,
                via: Vec::new(),
                scopes: Vec::new(),
            });
        }

        let mut drawn = Vec::new();
        for route in &config.routes {
            if scope
                .pipeline()
                .is_some_and(|name| !route.applies_to(Some(name)))
            {
                continue;
            }
            for from in &route.from {
                for to in &route.to {
                    if members
                        .is_some_and(|members| !members.contains(from) || !members.contains(to))
                    {
                        continue;
                    }
                    let pair = (agent_id(from), agent_id(to));
                    if drawn.contains(&pair) {
                        continue;
                    }
                    drawn.push(pair.clone());

                    // A barrier constrains only the upstreams it names. Any other permitted
                    // sender wakes the agent directly, leaving parked flights untouched, so
                    // labelling that edge with the join condition would state the opposite of
                    // what happens.
                    // A spawn is checked first: it opens a new itinerary, so a barrier on the
                    // receiver cannot apply to it -- validation rejects that combination outright.
                    let (style, label) = if route.is_spawn() {
                        (EdgeStyle::Spawn, Some("spawn".to_owned()))
                    } else {
                        match graph.join_for(to) {
                            Some(spec) if spec.upstreams.contains(from) => (
                                EdgeStyle::Joined,
                                Some(
                                    match spec.join {
                                        Join::All => "all",
                                        Join::Any => "any",
                                    }
                                    .to_owned(),
                                ),
                            ),
                            Some(_) => (EdgeStyle::Bypass, None),
                            None => (EdgeStyle::Plain, None),
                        }
                    };

                    let (from_layer, to_layer) = (self.layer_of(&pair.0), self.layer_of(&pair.1));
                    let beside = from_layer == to_layer && pair.0 != pair.1;
                    let back = from_layer > to_layer || (from_layer == to_layer && !beside);
                    self.edges.push(Edge {
                        from: pair.0,
                        to: pair.1,
                        label,
                        style,
                        back,
                        both: false,
                        beside,
                        bulge: None,
                        from_y: 0.0,
                        to_y: 0.0,
                        gutter: None,
                        hook_x: None,
                        exit_x: None,
                        floor: None,
                        via: Vec::new(),
                        scopes: scopes
                            .get(&(from.clone(), to.clone()))
                            .cloned()
                            .unwrap_or_default(),
                    });
                }
            }
        }
    }

    /// Which column a node is in, or zero if it is not placed.
    fn layer_of(&self, id: &str) -> usize {
        self.node(id).map_or(0, |node| node.layer)
    }

    /// Routes the return paths, meets every edge to its nodes, and computes the overall extent.
    fn size(&mut self) {
        let tallest = self
            .nodes
            .iter()
            .map(|node| node.y + node.h)
            .chain(
                self.edges
                    .iter()
                    .flat_map(|edge| edge.via.iter().map(|(_, y)| y + stack::THROUGH_H / 2.0)),
            )
            .fold(0.0_f64, f64::max);

        let (deepest, exits) = self.route_returns();
        self.assign_ports();
        let beside = tidy::route_beside(&self.nodes, &mut self.edges);

        self.width = self
            .nodes
            .iter()
            .map(|node| node.x + node.w)
            .fold(beside.max(exits), f64::max)
            + MARGIN;
        self.height = tallest.max(deepest) + MARGIN;
    }
}

/// The pipelines each drawn pair is scoped to, for the whole-factory map.
///
/// A pair several routes permit is drawn once, so its scopes are the union of theirs — and a
/// pair any global route permits is every workflow's, which is recorded as no scopes at all.
pub(crate) fn edge_scopes(config: &Config) -> BTreeMap<(AgentName, AgentName), Vec<String>> {
    let mut found: BTreeMap<(AgentName, AgentName), Option<BTreeSet<String>>> = BTreeMap::new();

    for route in &config.routes {
        for from in &route.from {
            for to in &route.to {
                let entry = found
                    .entry((from.clone(), to.clone()))
                    .or_insert_with(|| Some(BTreeSet::new()));
                match (&route.pipelines, entry.as_mut()) {
                    (None, _) => *entry = None,
                    (Some(names), Some(scopes)) => {
                        scopes.extend(names.iter().map(ToString::to_string));
                    }
                    (Some(_), None) => {}
                }
            }
        }
    }

    found
        .into_iter()
        .map(|(pair, scopes)| {
            (
                pair,
                scopes.map(|s| s.into_iter().collect()).unwrap_or_default(),
            )
        })
        .collect()
}

/// The left edge of a column.
fn column_x(layer: usize) -> f64 {
    MARGIN + precise(layer) * (NODE_W + COL_GAP)
}

/// Where the `slot`-th of `count` edges should meet a node's side.
///
/// Evenly spaced across the middle 70% of the height, so a single edge still meets the centre and
/// several never reach the rounded corners.
fn port(top: f64, height: f64, slot: usize, count: usize) -> f64 {
    if count <= 1 {
        return top + height / 2.0;
    }
    let usable = height * 0.7;
    let step = usable / precise(count - 1);
    top + (height - usable) / 2.0 + step * precise(slot)
}

/// Widens a count to a float for geometry.
///
/// Diagrams have tens of nodes, not quadrillions, so the lossy cast clippy warns about cannot
/// happen here. Saying so once in a named function is better than scattering allow attributes
/// through the arithmetic.
fn precise(count: usize) -> f64 {
    u32::try_from(count).map_or(f64::from(u32::MAX), f64::from)
}

/// A stable identifier for an agent node.
pub(super) fn agent_id(name: &AgentName) -> String {
    format!("a_{name}")
}

/// A stable identifier for a pipeline node.
pub(super) fn pipeline_id(name: &PipelineName) -> String {
    format!("p_{name}")
}

#[cfg(test)]
mod tests;
