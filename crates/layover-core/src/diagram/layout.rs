//! Deciding where each node goes.
//!
//! A layered layout, which is the right shape for a route map: work flows from an entry point
//! towards a terminal agent, and putting each node one column further right than the thing that
//! wakes it makes that flow the diagram's primary axis.
//!
//! # Layers come from breadth-first distance, not longest path
//!
//! The textbook layered algorithm assigns layers by longest path from a source, which does not
//! terminate on a cyclic graph. Route maps are routinely cyclic — a developer sends to a tester
//! and the tester sends back, which is the review loop that makes the reference factory work —
//! so longest path is not available.
//!
//! Breadth-first distance is, and [`crate::graph::RouteGraph`] already computes it for the
//! load-time hop check. Reusing it means the diagram's columns and the validator's hop arithmetic
//! are derived from the same number, so a diagram can never imply a depth that validation
//! disagrees with.
//!
//! Edges that point backwards or sideways are then drawn as return paths, which is what they are.

use std::collections::BTreeMap;

use crate::agent::{Access, AgentName};
use crate::config::Config;
use crate::diagram::{Activity, Live};
use crate::graph::RouteGraph;
use crate::pipeline::PipelineName;
use crate::route::Join;

/// Width of a node box.
const NODE_W: f64 = 168.0;
/// Height of a node box.
const NODE_H: f64 = 56.0;
/// Horizontal gap between columns.
const COL_GAP: f64 = 96.0;
/// Vertical gap between nodes in a column.
const ROW_GAP: f64 = 32.0;
/// Margin around the whole drawing.
const MARGIN: f64 = 32.0;
/// Vertical spacing between the lanes that return paths are routed through.
const RETURN_GAP: f64 = 26.0;

/// What a node represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A way into the mesh.
    Pipeline,
    /// A configured agent.
    Agent,
}

/// How a node is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// An ordinary agent or pipeline.
    Box,
    /// An agent guarded by a rendezvous barrier. Drawn as a gate, because that is what it is.
    Gate,
}

/// A positioned node.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Stable identifier, used to join edges to nodes and for DOM ids.
    pub id: String,
    /// The name shown on the node.
    pub label: String,
    /// A second line: a trigger for a pipeline, `read-only` for an agent that has it.
    pub subtitle: Option<String>,
    /// What this node represents.
    pub kind: NodeKind,
    /// How to draw it.
    pub shape: Shape,
    /// What it is doing, if anything.
    pub activity: Option<Activity>,
    /// Which column it sits in.
    pub layer: usize,
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

impl Node {
    /// The point an edge should leave from.
    #[must_use]
    pub fn exit(&self) -> (f64, f64) {
        (self.x + self.w, self.y + self.h / 2.0)
    }

    /// The point an edge should arrive at.
    #[must_use]
    pub fn entry(&self) -> (f64, f64) {
        (self.x, self.y + self.h / 2.0)
    }

    /// The centre.
    #[must_use]
    pub fn centre(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// How an edge is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeStyle {
    /// An ordinary permitted edge.
    Plain,
    /// A pipeline feeding its entry agent.
    Entry,
    /// One of the upstreams a barrier names.
    Joined,
    /// A permitted sender that the barrier does *not* name, and which therefore wakes the agent
    /// directly rather than parking at it.
    Bypass,
    /// Opens a new itinerary per flight rather than continuing this one.
    Spawn,
}

/// A positioned edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    /// Identifier of the node it leaves.
    pub from: String,
    /// Identifier of the node it arrives at.
    pub to: String,
    /// `all` or `any`, for an edge into a barrier.
    pub label: Option<String>,
    /// How to draw it.
    pub style: EdgeStyle,
    /// True when the edge points back towards the entry, which makes it a return path.
    pub back: bool,
    /// For a return path, the depth it dips to. `None` for a forward edge.
    ///
    /// Computed here rather than in the renderer because it decides how tall the drawing is, and
    /// a renderer that invented its own geometry would draw outside the reported extent — which
    /// is exactly how a review loop ends up clipped off the bottom of the diagram.
    pub floor: Option<f64>,
}

/// A laid-out diagram.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    /// Every node, in reading order.
    pub nodes: Vec<Node>,
    /// Every edge.
    pub edges: Vec<Edge>,
    /// Total width, including margins.
    pub width: f64,
    /// Total height, including margins.
    pub height: f64,
}

impl Layout {
    /// Finds a node by identifier.
    #[must_use]
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// Lays out a factory.
    #[must_use]
    pub fn build(config: &Config, live: &Live) -> Self {
        let graph = RouteGraph::from_config(config);
        let layers = assign_layers(config, &graph);

        let mut layout = Self::default();
        layout.place(config, live, &graph, &layers);
        layout.connect(config, &graph);
        layout.order_by_barycentre();
        layout.size();
        layout
    }

    /// Creates a node for every pipeline and agent, and puts it in its column.
    fn place(
        &mut self,
        config: &Config,
        live: &Live,
        graph: &RouteGraph,
        layers: &BTreeMap<AgentName, usize>,
    ) {
        let mut columns: BTreeMap<usize, Vec<Node>> = BTreeMap::new();

        for (name, pipeline) in &config.pipelines {
            columns.entry(0).or_default().push(Node {
                id: pipeline_id(name),
                label: name.as_str().to_owned(),
                subtitle: Some(pipeline.trigger.to_string()),
                kind: NodeKind::Pipeline,
                shape: Shape::Box,
                activity: None,
                layer: 0,
                x: 0.0,
                y: 0.0,
                w: NODE_W,
                h: NODE_H,
            });
        }

        for (name, agent) in &config.agents {
            // Agents no pipeline can reach still have to appear — an unreachable agent is
            // precisely the thing somebody opened the diagram to find.
            let layer = layers.get(name).copied().unwrap_or(0) + 1;
            columns.entry(layer).or_default().push(Node {
                id: agent_id(name),
                label: name.as_str().to_owned(),
                subtitle: (agent.access == Access::ReadOnly).then(|| "read-only".to_owned()),
                kind: NodeKind::Agent,
                shape: if graph.join_for(name).is_some() {
                    Shape::Gate
                } else {
                    Shape::Box
                },
                activity: live.activity.get(name).copied(),
                layer,
                x: 0.0,
                y: 0.0,
                w: NODE_W,
                h: NODE_H,
            });
        }

        for (layer, mut nodes) in columns {
            let x = MARGIN + precise(layer) * (NODE_W + COL_GAP);
            for (row, node) in nodes.iter_mut().enumerate() {
                node.x = x;
                node.y = MARGIN + precise(row) * (NODE_H + ROW_GAP);
            }
            self.nodes.append(&mut nodes);
        }
    }

    /// Adds the edges, classifying each one.
    fn connect(&mut self, config: &Config, graph: &RouteGraph) {
        for (name, pipeline) in &config.pipelines {
            self.edges.push(Edge {
                from: pipeline_id(name),
                to: agent_id(&pipeline.entry),
                label: None,
                style: EdgeStyle::Entry,
                back: false,
                floor: None,
            });
        }

        let mut drawn = Vec::new();
        for route in &config.routes {
            for from in &route.from {
                for to in &route.to {
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

                    let back = self.layer_of(&pair.0) >= self.layer_of(&pair.1);
                    self.edges.push(Edge {
                        from: pair.0,
                        to: pair.1,
                        label,
                        style,
                        back,
                        floor: None,
                    });
                }
            }
        }
    }

    /// Which column a node is in, or zero if it is not placed.
    fn layer_of(&self, id: &str) -> usize {
        self.node(id).map_or(0, |node| node.layer)
    }

    /// Reorders each column to sit near the things that point at it.
    ///
    /// One pass of the barycentre heuristic. It is not optimal — crossing minimisation is
    /// NP-hard — but on a graph of this size one pass removes most of the obvious tangles, and a
    /// second pass tends to shuffle nodes without improving anything a human would notice.
    fn order_by_barycentre(&mut self) {
        let positions: BTreeMap<String, f64> = self
            .nodes
            .iter()
            .map(|node| (node.id.clone(), node.centre().1))
            .collect();

        let mut keys: BTreeMap<String, (f64, String)> = BTreeMap::new();
        for node in &self.nodes {
            let incoming: Vec<f64> = self
                .edges
                .iter()
                .filter(|edge| edge.to == node.id && !edge.back)
                .filter_map(|edge| positions.get(&edge.from).copied())
                .collect();

            // No incoming edges leaves the node where it was: its own position is the only
            // information available, and inventing an order would be churn.
            let bary = if incoming.is_empty() {
                node.centre().1
            } else {
                incoming.iter().sum::<f64>() / precise(incoming.len())
            };
            keys.insert(node.id.clone(), (bary, node.label.clone()));
        }

        let mut by_layer: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (index, node) in self.nodes.iter().enumerate() {
            by_layer.entry(node.layer).or_default().push(index);
        }

        for indices in by_layer.into_values() {
            let mut ordered = indices.clone();
            ordered.sort_by(|left, right| {
                let a = &keys[&self.nodes[*left].id];
                let b = &keys[&self.nodes[*right].id];
                a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1))
            });

            let ys: Vec<f64> = indices.iter().map(|i| self.nodes[*i].y).collect();
            for (slot, index) in ordered.into_iter().enumerate() {
                self.nodes[index].y = ys[slot];
            }
        }
    }

    /// Computes the overall extent, centres each column vertically, and routes the return paths.
    fn size(&mut self) {
        let tallest = self
            .nodes
            .iter()
            .map(|node| node.y + node.h)
            .fold(0.0_f64, f64::max);

        let mut bottoms: BTreeMap<usize, f64> = BTreeMap::new();
        for node in &self.nodes {
            let bottom = bottoms.entry(node.layer).or_insert(0.0);
            *bottom = bottom.max(node.y + node.h);
        }
        for node in &mut self.nodes {
            node.y += (tallest - bottoms[&node.layer]) / 2.0;
        }

        let deepest = self.route_returns(tallest);

        self.width = self
            .nodes
            .iter()
            .map(|node| node.x + node.w)
            .fold(0.0_f64, f64::max)
            + MARGIN;
        self.height = tallest.max(deepest) + MARGIN;
    }

    /// Gives each return path its own lane below the drawing, and reports the deepest one.
    ///
    /// Lanes rather than one shared depth: two loops at the same height would be drawn on top of
    /// each other, and a review loop is the structure on a route map most worth being able to
    /// follow with a finger.
    fn route_returns(&mut self, floor_start: f64) -> f64 {
        let mut lanes: Vec<(String, String)> = self
            .edges
            .iter()
            .filter(|edge| edge.back)
            .map(|edge| (edge.from.clone(), edge.to.clone()))
            .collect();
        // Shortest loops innermost, so a long return path never has to cross a short one.
        lanes.sort_by_key(|(from, to)| {
            let span = self
                .node(from)
                .zip(self.node(to))
                .map_or(0, |(f, t)| f.layer.abs_diff(t.layer));
            (span, from.clone(), to.clone())
        });

        let mut deepest = floor_start;
        for (index, key) in lanes.iter().enumerate() {
            let depth = floor_start + RETURN_GAP * (precise(index) + 1.0);
            deepest = deepest.max(depth);
            if let Some(edge) = self
                .edges
                .iter_mut()
                .find(|edge| edge.back && (edge.from.clone(), edge.to.clone()) == *key)
            {
                edge.floor = Some(depth);
            }
        }
        deepest
    }
}

/// Assigns each agent a column by breadth-first distance from the ways in.
///
/// Agents nothing can reach are absent from the result, and the caller places them in the first
/// column rather than dropping them: an unreachable agent is exactly what somebody opened the
/// diagram to find, so hiding it would defeat the purpose.
fn assign_layers(config: &Config, graph: &RouteGraph) -> BTreeMap<AgentName, usize> {
    let sources: Vec<AgentName> = config
        .pipelines
        .values()
        .map(|pipeline| pipeline.entry.clone())
        .chain(
            config
                .agents
                .iter()
                .filter(|(_, agent)| agent.entry)
                .map(|(name, _)| name.clone()),
        )
        .collect();

    graph
        .distances_from(sources.iter())
        .into_iter()
        .map(|(name, distance)| (name, distance as usize))
        .collect()
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
fn agent_id(name: &AgentName) -> String {
    format!("a_{name}")
}

/// A stable identifier for a pipeline node.
fn pipeline_id(name: &PipelineName) -> String {
    format!("p_{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(body: &str) -> Config {
        Config::from_toml(body, "layout-test.toml").expect("config parses")
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

            [agents.developer]
            prompt = "develop"

            [agents.tester]
            prompt = "test"
            access = "read-only"

            [agents.publisher]
            prompt = "publish"

            [pipelines.triage]
            entry = "analyst"

            [[routes]]
            from = "analyst"
            to = "developer"

            [[routes]]
            from = "developer"
            to = "tester"

            [[routes]]
            from = "tester"
            to = "developer"
            join = "all"

            [[routes]]
            from = "developer"
            to = "publisher"
            "#,
        )
    }

    #[test]
    fn work_flows_left_to_right_one_column_per_hop() {
        let layout = Layout::build(&factory(), &Live::default());

        assert_eq!(layout.node("p_triage").expect("pipeline").layer, 0);
        assert_eq!(layout.node("a_analyst").expect("analyst").layer, 1);
        assert_eq!(layout.node("a_developer").expect("developer").layer, 2);
        assert_eq!(layout.node("a_tester").expect("tester").layer, 3);
    }

    #[test]
    fn a_loop_does_not_hang_the_layout() {
        // Longest-path layering, the textbook approach, does not terminate here: the developer
        // and the tester point at each other, which is the review loop the reference factory is
        // built around. Breadth-first distance is what makes cyclic route maps drawable at all.
        let layout = Layout::build(&factory(), &Live::default());

        assert_eq!(layout.nodes.len(), 5);
        assert!(layout.width > 0.0 && layout.height > 0.0);
    }

    #[test]
    fn an_edge_pointing_back_towards_the_entry_is_marked_as_a_return_path() {
        let layout = Layout::build(&factory(), &Live::default());

        let back = layout
            .edges
            .iter()
            .find(|edge| edge.from == "a_tester" && edge.to == "a_developer")
            .expect("the review loop exists");

        assert!(back.back, "tester sits right of developer, so this returns");
        assert_eq!(back.style, EdgeStyle::Joined);
        assert_eq!(back.label.as_deref(), Some("all"));
    }

    #[test]
    fn a_sender_the_barrier_does_not_name_is_marked_as_bypassing_it() {
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

        let layout = Layout::build(&config, &Live::default());
        let bypass = layout
            .edges
            .iter()
            .find(|edge| edge.from == "a_scanner" && edge.to == "a_collector")
            .expect("scanner may send to collector");

        assert_eq!(bypass.style, EdgeStyle::Bypass);
        assert_eq!(
            bypass.label, None,
            "it does not wait, so it has no condition"
        );
    }

    #[test]
    fn a_joined_agent_is_drawn_as_a_gate() {
        let layout = Layout::build(&factory(), &Live::default());

        assert_eq!(
            layout.node("a_developer").expect("developer").shape,
            Shape::Gate
        );
        assert_eq!(layout.node("a_analyst").expect("analyst").shape, Shape::Box);
    }

    #[test]
    fn nodes_in_a_column_never_overlap() {
        let layout = Layout::build(&factory(), &Live::default());

        for layer in 0..4 {
            let mut boxes: Vec<(f64, f64)> = layout
                .nodes
                .iter()
                .filter(|node| node.layer == layer)
                .map(|node| (node.y, node.y + node.h))
                .collect();
            boxes.sort_by(|a, b| a.0.total_cmp(&b.0));

            for pair in boxes.windows(2) {
                assert!(
                    pair[1].0 >= pair[0].1,
                    "layer {layer} has overlapping nodes: {pair:?}"
                );
            }
        }
    }

    #[test]
    fn every_node_sits_inside_the_reported_extent() {
        // The extent becomes the SVG viewBox. A node outside it is a node nobody can see.
        let layout = Layout::build(&factory(), &Live::default());

        for node in &layout.nodes {
            assert!(node.x >= 0.0 && node.y >= 0.0, "{} is off-canvas", node.id);
            assert!(node.x + node.w <= layout.width, "{} overflows", node.id);
            assert!(node.y + node.h <= layout.height, "{} overflows", node.id);
        }
    }

    #[test]
    fn return_paths_sit_inside_the_reported_extent_too() {
        // The first version of this reserved height for nodes only, and every review loop in the
        // reference factory was drawn below the viewBox and clipped away. Found by rendering it
        // and looking, which is the only way that class of bug ever shows up.
        let layout = Layout::build(&factory(), &Live::default());

        let returns: Vec<&Edge> = layout.edges.iter().filter(|edge| edge.back).collect();
        assert!(!returns.is_empty(), "the factory has a review loop to test");

        for edge in returns {
            let floor = edge.floor.expect("a return path is given a lane");
            assert!(
                floor <= layout.height,
                "{} -> {} dips to {floor} but the drawing is only {} tall",
                edge.from,
                edge.to,
                layout.height
            );
        }
    }

    #[test]
    fn two_return_paths_are_given_lanes_of_their_own() {
        // Drawn at the same depth they would overlap, and a review loop is the structure on a
        // route map most worth being able to follow with a finger.
        let config = config(
            r#"
            [layover]
            work_dir = "work"

            [defaults]
            runner = "claude"

            [runners.claude]
            command = ["claude", "-p"]

            [agents.dev]
            prompt = "develop"

            [agents.tester]
            prompt = "test"

            [agents.reviewer]
            prompt = "review"

            [pipelines.go]
            entry = "dev"

            [[routes]]
            from = "dev"
            to = ["tester", "reviewer"]

            [[routes]]
            from = ["tester", "reviewer"]
            to = "dev"
            join = "all"
            "#,
        );

        let layout = Layout::build(&config, &Live::default());
        let mut floors: Vec<f64> = layout
            .edges
            .iter()
            .filter(|edge| edge.back)
            .filter_map(|edge| edge.floor)
            .collect();
        floors.sort_by(f64::total_cmp);

        assert_eq!(floors.len(), 2);
        assert!(
            (floors[1] - floors[0]).abs() > 1.0,
            "the two loops share a lane: {floors:?}"
        );
    }

    #[test]
    fn an_agent_no_pipeline_can_reach_is_still_drawn() {
        // An unreachable agent is exactly what somebody opens the diagram to find, so dropping
        // it would defeat the purpose of drawing one.
        let config = config(
            r#"
            [layover]
            work_dir = "work"

            [defaults]
            runner = "claude"

            [runners.claude]
            command = ["claude", "-p"]

            [agents.reachable]
            prompt = "work"

            [agents.orphan]
            prompt = "nobody routes here"

            [pipelines.go]
            entry = "reachable"
            "#,
        );

        let layout = Layout::build(&config, &Live::default());

        assert!(layout.node("a_orphan").is_some());
    }

    #[test]
    fn live_state_lands_on_the_right_node() {
        let live = Live::default().with("developer", Activity::Running);
        let layout = Layout::build(&factory(), &live);

        assert_eq!(
            layout.node("a_developer").expect("developer").activity,
            Some(Activity::Running)
        );
        assert_eq!(layout.node("a_analyst").expect("analyst").activity, None);
    }

    #[test]
    fn an_empty_factory_lays_out_without_panicking() {
        let config = config(
            r#"
            [layover]
            work_dir = "work"

            [defaults]
            runner = "claude"

            [runners.claude]
            command = ["claude", "-p"]

            [agents.only]
            prompt = "think"
            entry = true
            "#,
        );

        let layout = Layout::build(&config, &Live::default());

        assert_eq!(layout.nodes.len(), 1);
        assert!(layout.edges.is_empty());
    }
}
