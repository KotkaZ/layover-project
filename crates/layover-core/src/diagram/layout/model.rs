//! What a laid-out route map is made of: positioned nodes and the edges between them.

use std::collections::BTreeSet;

use crate::diagram::Activity;

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
    /// A second line: a trigger for a pipeline; for an agent, the model its command line names
    /// and the effort it runs it at, or failing both its context tier and `read-only`.
    pub subtitle: Option<String>,
    /// A third line, for an agent whose model took the second: its context tier and `read-only`.
    pub caption: Option<String>,
    /// A third small line, for an agent whose model name is too long to share its line with the
    /// effort, so that nothing is cut off to make room.
    pub footnote: Option<String>,
    /// Everything worth knowing about the node, one fact per line, for hovering over it.
    pub tooltip: Option<String>,
    /// What this node represents.
    pub kind: NodeKind,
    /// How to draw it.
    pub shape: Shape,
    /// What it is doing, if anything.
    pub activity: Option<Activity>,
    /// A count drawn in its corner, such as `×2` for an agent that ran twice in a chain.
    pub badge: Option<String>,
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
    ///
    /// Strictly backwards: an edge between two agents in the same column is [`Self::beside`]
    /// instead, because a loop under the whole drawing is a long way round for two neighbours.
    pub back: bool,
    /// True when one line stands for a route in each direction, drawn with an arrowhead at both
    /// ends. Only two plain routes are merged: a join or a spawn says something the other
    /// direction does not, and so does a scope that differs.
    pub both: bool,
    /// True when both ends sit in the same column.
    pub beside: bool,
    /// For an edge [`Self::beside`] its column between agents that are not next to each other,
    /// how far right of the column it swings out. `None` for neighbours, which are joined by a
    /// straight connector through the gap between them.
    pub bulge: Option<f64>,
    /// Where on the source's right edge this leaves, as an absolute y.
    ///
    /// Edges all left from the node's centre, so several going to different places overlapped
    /// for their first stretch and only separated once they had already crossed each other.
    /// Spreading them down the edge, ordered by where they are going, means they never cross at
    /// the node they share.
    pub from_y: f64,
    /// Where on the target's left edge this arrives, as an absolute y.
    pub to_y: f64,
    /// For a forward edge that skips columns, the points it runs through: for each column it
    /// crosses, where it enters that column and where it leaves it, at a height the layout keeps
    /// clear of every box there. Empty for an edge between neighbouring columns.
    ///
    /// A curve drawn straight from one column to a column three further on ran through whatever
    /// boxes stood between, and a line through a box reads as a route to it.
    pub via: Vec<(f64, f64)>,
    /// For a return path, the x it climbs at; for a connector between neighbours in one column,
    /// the x it runs at. `None` for a forward edge.
    ///
    /// Distinct per edge even when several return to the same agent. Sharing one gutter put four
    /// curves on the same vertical line with their labels stacked on top of each other.
    pub gutter: Option<f64>,
    /// For a return path, where it hooks into the target's underside.
    pub hook_x: Option<f64>,
    /// For a return path, the x it drops at, in the gap right of the column it leaves.
    ///
    /// Dropping straight down from the source ran through every box below it in the same column,
    /// and between the boxes the line looked like a route between neighbours.
    pub exit_x: Option<f64>,
    /// For a return path, the depth it dips to. `None` for a forward edge.
    ///
    /// Computed here rather than in the renderer because it decides how tall the drawing is, and
    /// a renderer that invented its own geometry would draw outside the reported extent — which
    /// is exactly how a review loop ends up clipped off the bottom of the diagram.
    pub floor: Option<f64>,
    /// The pipelines this edge is scoped to, sorted, on the whole-factory map.
    ///
    /// Empty when a global route permits it, because every workflow may use it — and always empty
    /// on one workflow's map, where every edge drawn is that workflow's by construction.
    pub scopes: Vec<String>,
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
    /// On one chain's map, the drawn edges its work took, by the identifiers of their two ends as
    /// drawn. Empty on a workflow's map.
    pub travelled: BTreeSet<(String, String)>,
}

impl Layout {
    /// Finds a node by identifier.
    #[must_use]
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// Whether the chain this map is drawn for took `edge`.
    #[must_use]
    pub fn took(&self, edge: &Edge) -> bool {
        self.travelled
            .iter()
            .any(|(from, to)| *from == edge.from && *to == edge.to)
    }
}
