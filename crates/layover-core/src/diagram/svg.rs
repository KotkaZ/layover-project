//! Turning a [`Layout`] into SVG.
//!
//! Inline SVG rather than a canvas or a JavaScript graph library: it needs no runtime, renders
//! before any script has parsed, survives the browser's zoom, and every node is a DOM element
//! that CSS can colour and a pointer can hit. For a diagram of a few dozen nodes there is nothing
//! a library would add except its own weight.

use std::fmt::Write as _;

use crate::diagram::caption;
use crate::diagram::layout::{Edge, EdgeStyle, Layout, Node, NodeKind, Shape};

/// How far below a node the hook begins.
const HOOK: f64 = 26.0;
/// Radius of the rounded corners where a return path turns into and out of its lane.
const BEND: f64 = 14.0;
/// How far in from its source's right-hand corner a return path leaves the underside.
const LEAVE_INSET: f64 = 18.0;
/// How far below its source a return path turns towards the gap right of the column.
const SHELF: f64 = 10.0;
/// How far above the lane an edge label sits.
const LABEL_LIFT: f64 = 6.0;

/// Renders a layout as an SVG fragment.
///
/// A fragment, not a document: it is embedded in the dashboard page, so it inherits the page's
/// styles and font. The `viewBox` makes it scale to whatever box it is put in.
#[must_use]
pub fn render(layout: &Layout) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg class="routemap" viewBox="0 0 {:.0} {:.0}" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Factory route map">"#,
        layout.width, layout.height
    );

    out.push_str(ARROWHEADS);

    // Edges first so that nodes sit on top of them: a line ending under a box reads as arriving
    // at it, whereas a box under a line reads as being crossed out.
    for edge in &layout.edges {
        render_edge(&mut out, layout, edge);
    }
    for node in &layout.nodes {
        render_node(&mut out, node);
    }

    out.push_str("</svg>\n");
    out
}

/// Marker definitions, one per edge style so each arrowhead matches its line.
const ARROWHEADS: &str = r#"  <defs>
    <marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 0 L 10 5 L 0 10 z" class="arrowhead"/>
    </marker>
    <marker id="arrow-bypass" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 0 L 10 5 L 0 10 z" class="arrowhead bypass"/>
    </marker>
  </defs>
"#;

/// Draws one node, with its label and whatever it is doing.
fn render_node(out: &mut String, node: &Node) {
    let kind = match node.kind {
        NodeKind::Pipeline => "pipeline",
        NodeKind::Agent => "agent",
    };
    let state = node
        .activity
        .map_or(String::new(), |activity| format!(" {}", activity.class()));

    let _ = writeln!(
        out,
        r#"  <g class="node {kind}{state}" id="{}" tabindex="0">"#,
        escape(&node.id)
    );
    // First, so assistive technology announces it as the node's name and a pointer shows it.
    if let Some(tooltip) = &node.tooltip {
        let _ = writeln!(out, "    <title>{}</title>", escape(tooltip));
    }

    match node.shape {
        Shape::Box => {
            let _ = writeln!(
                out,
                r#"    <rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="10"/>"#,
                node.x, node.y, node.w, node.h
            );
        }
        Shape::Gate => {
            // A hexagon reads as a gate, which is what a barrier is: work queues at it until a
            // condition is met. A plain box would make it look like any other agent.
            let notch = 14.0;
            let (x, y, w, h) = (node.x, node.y, node.w, node.h);
            let _ = writeln!(
                out,
                r#"    <polygon points="{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} {:.1},{:.1} {:.1},{:.1} {:.1},{:.1}"/>"#,
                x + notch,
                y,
                x + w - notch,
                y,
                x + w,
                y + h / 2.0,
                x + w - notch,
                y + h,
                x + notch,
                y + h,
                x,
                y + h / 2.0
            );
        }
    }

    let (cx, cy) = node.centre();
    let (baseline, sub_at) = match (&node.subtitle, &node.caption, &node.footnote) {
        (_, _, Some(_)) => (cy - 16.0, cy),
        (_, Some(_), None) => (cy - 10.0, cy + 6.0),
        (Some(_), None, None) => (cy - 4.0, cy + 13.0),
        (None, None, None) => (cy + 5.0, cy + 13.0),
    };
    let _ = writeln!(
        out,
        r#"    <text class="label" x="{cx:.1}" y="{baseline:.1}" text-anchor="middle">{}</text>"#,
        escape(&node.label)
    );
    if let Some(subtitle) = &node.subtitle {
        let _ = writeln!(
            out,
            r#"    <text class="sub" x="{cx:.1}" y="{sub_at:.1}" text-anchor="middle">{}</text>"#,
            escape(subtitle)
        );
    }
    for (line, below) in [(&node.caption, 13.0), (&node.footnote, 26.0)] {
        if let Some(line) = line {
            let _ = writeln!(
                out,
                r#"    <text class="sub caption" x="{cx:.1}" y="{:.1}" text-anchor="middle">{}</text>"#,
                sub_at + below,
                escape(line)
            );
        }
    }

    out.push_str("  </g>\n");
}

/// Draws one edge, as a curve from the right of its source to the left of its target.
fn render_edge(out: &mut String, layout: &Layout, edge: &Edge) {
    let (Some(from), Some(to)) = (layout.node(&edge.from), layout.node(&edge.to)) else {
        return;
    };

    let class = match edge.style {
        EdgeStyle::Plain => "edge",
        EdgeStyle::Entry => "edge entry",
        EdgeStyle::Joined => "edge joined",
        EdgeStyle::Bypass => "edge bypass",
        EdgeStyle::Spawn => "edge spawn",
    };
    let marker = if matches!(edge.style, EdgeStyle::Bypass | EdgeStyle::Spawn) {
        "arrow-bypass"
    } else {
        "arrow"
    };

    let (path, label_at) = if edge.beside {
        beside_path(from, to, edge)
    } else if edge.back {
        return_path(from, to, edge)
    } else {
        forward_path(from, to, edge.from_y, edge.to_y)
    };

    // A two-way line carries the same arrowhead at its start, which `auto-start-reverse` turns to
    // point back into the node the line leaves.
    let start = if edge.both {
        format!(r#" marker-start="url(#{marker})""#)
    } else {
        String::new()
    };

    // One group per route, naming both ends, so a page can light an agent's routes without
    // knowing anything about the layout. A scoped edge on the whole-factory map is classed so a
    // stylesheet can pick it out; its tooltip says which workflows may use it. The wide, invisible
    // `hit` path is what a pointer actually lands on — a line 1.6 pixels wide is hard to hover.
    let scoped = if edge.scopes.is_empty() {
        ""
    } else {
        " scoped"
    };
    let both = if edge.both { " both" } else { "" };
    let _ = writeln!(
        out,
        r#"  <g class="route{both}{scoped}" data-from="{}" data-to="{}">"#,
        escape(&edge.from),
        escape(&edge.to)
    );
    let _ = writeln!(
        out,
        "    <title>{}</title>",
        escape(&caption::route(&from.label, &to.label, edge))
    );
    let _ = writeln!(out, r#"    <path class="hit" d="{path}"/>"#);
    let _ = writeln!(
        out,
        r#"    <path class="{class}{scoped}" d="{path}"{start} marker-end="url(#{marker})"/>"#
    );

    if let Some(label) = &edge.label {
        let _ = writeln!(
            out,
            r#"    <text class="edgelabel" x="{:.1}" y="{:.1}" text-anchor="middle">{}</text>"#,
            label_at.0,
            label_at.1,
            escape(label)
        );
    }
    out.push_str("  </g>\n");
}

/// A cubic curve rightwards, flattening into a straight line when the ends are level.
///
/// The y of each end comes from the layout rather than the node centre, so several edges sharing
/// a node leave and arrive at different points along its side instead of bunching.
fn forward_path(from: &Node, to: &Node, from_y: f64, to_y: f64) -> (String, (f64, f64)) {
    let (x1, y1) = (from.exit().0, from_y);
    let (x2, y2) = (to.entry().0, to_y);
    let bend = ((x2 - x1) * 0.45).max(24.0);

    (
        format!(
            "M {x1:.1} {y1:.1} C {:.1} {y1:.1} {:.1} {y2:.1} {x2:.1} {y2:.1}",
            x1 + bend,
            x2 - bend
        ),
        (f64::midpoint(x1, x2), f64::midpoint(y1, y2) - 8.0),
    )
}

/// An edge between two agents in the same column, drawn beside it.
///
/// Neighbours are joined by a straight connector through the gap between them. Agents further
/// apart get an arc out to the right that leaves and arrives near the ends facing each other, so
/// it hugs the column instead of cutting across the boxes between them. The layout decides how far
/// out it swings, which is what keeps it inside the drawing and out of the next column.
fn beside_path(from: &Node, to: &Node, edge: &Edge) -> (String, (f64, f64)) {
    let downwards = from.y < to.y;

    if let Some(bulge) = edge.bulge {
        let x = from.x + from.w;
        let (y1, y2) = if downwards {
            (from.y + from.h * 0.8, to.y + to.h * 0.2)
        } else {
            (from.y + from.h * 0.2, to.y + to.h * 0.8)
        };
        // Control points at 4/3 of the bulge put the curve's furthest point exactly at the bulge.
        let control = x + bulge * 4.0 / 3.0;
        return (
            format!("M {x:.1} {y1:.1} C {control:.1} {y1:.1} {control:.1} {y2:.1} {x:.1} {y2:.1}"),
            (x + bulge + 12.0, f64::midpoint(y1, y2)),
        );
    }

    let x = edge.gutter.unwrap_or_else(|| from.centre().0);
    let (y1, y2) = if downwards {
        (from.y + from.h, to.y)
    } else {
        (from.y, to.y + to.h)
    };
    (
        format!("M {x:.1} {y1:.1} L {x:.1} {y2:.1}"),
        (x + 16.0, f64::midpoint(y1, y2) + 4.0),
    )
}

/// A return path: out of the bottom, back leftwards through its own lane, and up into the
/// target's underside.
///
/// Drawn below everything rather than straight through the columns it crosses. A review loop is
/// the most important structure on a diagram like this, and routing it through the middle of the
/// forward flow is what makes these graphs unreadable.
///
/// The lane depth comes from the layout, which is what guarantees the curve stays inside the
/// reported extent instead of being clipped off the bottom.
fn return_path(from: &Node, to: &Node, edge: &Edge) -> (String, (f64, f64)) {
    let ty = to.y + to.h;
    let floor = edge.floor.unwrap_or((from.y + from.h).max(ty) + 34.0);

    // Climb in a gutter left of the target's column rather than straight up into its underside.
    // Columns stack several agents, so a vertical segment on the column centre runs through
    // whichever boxes sit below the target, and a line passing through a node reads as an edge
    // touching it.
    //
    // The gutter and the hook are per edge, not per target. Several returns to one agent — which
    // is the normal shape of a review loop — shared a single vertical line, so four curves
    // overlapped for their whole climb and their labels stacked on top of one another.
    let gutter = edge.gutter.unwrap_or_else(|| (to.x - 44.0).max(12.0));
    let corner = edge.hook_x.unwrap_or(to.x + 26.0);

    // Out from under the source's right-hand corner, into the gap right of its column, down that
    // gap, along the lane, up the gutter and into the target — straight runs with rounded
    // corners.
    //
    // Two earlier shapes failed. One cubic curve with its control points on the lane never reaches
    // the lane: its lowest point sits a quarter of the way short, so with a lane just below the
    // deepest box the curve ran behind that box. And dropping straight down from the source ran
    // through every box below it in the same column, showing between them as a short vertical line
    // that reads as a route between neighbours.
    let right = from.x + from.w;
    let bottom = from.y + from.h;
    let leave = right - LEAVE_INSET;
    let shelf = bottom + SHELF;
    let exit = edge.exit_x.unwrap_or(right + 20.0);
    let bend = BEND
        .min((exit - leave - SHELF) / 2.0)
        .min((floor - shelf) / 2.0)
        .min((exit - gutter).abs() / 2.0);
    (
        format!(
            "M {leave:.1} {bottom:.1} Q {leave:.1} {shelf:.1} {:.1} {shelf:.1} \
             L {:.1} {shelf:.1} Q {exit:.1} {shelf:.1} {exit:.1} {:.1} \
             L {exit:.1} {:.1} Q {exit:.1} {floor:.1} {:.1} {floor:.1} \
             L {:.1} {floor:.1} Q {gutter:.1} {floor:.1} {gutter:.1} {:.1} \
             L {gutter:.1} {:.1} \
             Q {gutter:.1} {ty:.1} {corner:.1} {ty:.1}",
            leave + SHELF,
            exit - bend,
            shelf + bend,
            floor - bend,
            exit - bend,
            gutter + bend,
            floor - bend,
            ty + HOOK
        ),
        (f64::midpoint(exit, gutter), floor - LABEL_LIFT),
    )
}

/// Escapes text going into SVG markup.
fn escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::diagram::{Activity, Live};

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
            access = "read-only"

            [agents.developer]
            prompt = "develop"

            [agents.tester]
            prompt = "test"

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
            "#,
            "svg-test.toml",
        )
        .expect("config parses")
    }

    fn svg() -> String {
        render(&Layout::build(&factory(), &Live::default()))
    }

    #[test]
    fn the_fragment_is_a_sized_svg_element() {
        let svg = svg();

        assert!(
            svg.starts_with(r#"<svg class="routemap" viewBox="0 0 "#),
            "{svg}"
        );
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn every_node_becomes_an_addressable_element() {
        // Each node is a DOM element with an id, which is what lets the page colour it, focus it
        // and hang a click on it without a graph library in the middle.
        let svg = svg();

        for id in ["p_triage", "a_analyst", "a_developer", "a_tester"] {
            assert!(svg.contains(&format!(r#"id="{id}""#)), "{id} missing");
        }
    }

    #[test]
    fn a_barrier_is_drawn_as_a_gate_and_an_ordinary_agent_as_a_box() {
        let svg = svg();

        assert!(
            svg.contains("<polygon"),
            "the joined developer needs a gate"
        );
        assert_eq!(svg.matches("<polygon").count(), 1);
        assert!(svg.contains("<rect"));
    }

    #[test]
    fn edges_are_drawn_before_nodes() {
        // A line ending under a box reads as arriving at it. A box under a line reads as being
        // crossed out.
        let svg = svg();
        let first_path = svg.find(r#"<path class="edge"#).expect("has edges");
        let first_node = svg.find(r#"<g class="node"#).expect("has nodes");

        assert!(first_path < first_node);
    }

    #[test]
    fn a_return_path_is_routed_below_rather_than_through_the_columns() {
        // The review loop is the most important structure on a route map, and drawing it through
        // the middle of the forward flow is what makes these diagrams unreadable.
        let layout = Layout::build(&factory(), &Live::default());
        let tester = layout.node("a_tester").expect("tester");
        let back = layout
            .edges
            .iter()
            .find(|edge| edge.back)
            .expect("the review loop exists");
        let (path, _) = return_path(tester, layout.node("a_developer").expect("developer"), back);

        let floor: f64 = path
            .split_whitespace()
            .filter_map(|token| token.parse::<f64>().ok())
            .fold(0.0, f64::max);

        assert!(
            floor > tester.y + tester.h,
            "the return path should dip below the nodes it passes"
        );
    }

    #[test]
    fn live_state_becomes_a_class_the_stylesheet_can_colour() {
        let live = Live::default()
            .with("developer", Activity::Running)
            .with("analyst", Activity::Failed);
        let svg = render(&Layout::build(&factory(), &live));

        assert!(
            svg.contains(r#"class="node agent running" id="a_developer""#),
            "{svg}"
        );
        assert!(svg.contains(r#"class="node agent failed" id="a_analyst""#));
        assert!(svg.contains(r#"class="node agent" id="a_tester""#));
    }

    #[test]
    fn a_join_condition_is_written_on_the_edge() {
        let svg = svg();

        assert!(svg.contains(r#"class="edgelabel""#));
        assert!(svg.contains(">all</text>"));
    }

    #[test]
    fn read_only_access_is_written_under_the_name() {
        let svg = svg();

        assert!(svg.contains(">read-only</text>"));
    }

    #[test]
    fn a_return_path_climbs_in_the_gutter_rather_than_through_the_column() {
        // Columns stack several agents, so a vertical segment on the column centre runs through
        // whichever boxes sit below the target, and a line passing through a node reads as an
        // edge touching it. Found by rendering the reference factory and looking at it: a return
        // path from the tester crossed the investigator and implied a route that does not exist.
        let layout = Layout::build(&factory(), &Live::default());
        let developer = layout.node("a_developer").expect("developer");
        let tester = layout.node("a_tester").expect("tester");
        let back = layout
            .edges
            .iter()
            .find(|edge| edge.back && edge.to == "a_developer")
            .expect("the review loop exists");
        let (path, _) = return_path(tester, developer, back);

        let gutter = back.gutter.expect("a return path is given a gutter");
        assert!(
            gutter < developer.x,
            "the ascent should happen left of the target's column: {gutter} vs {}",
            developer.x
        );
        assert!(path.contains(&format!("{gutter:.1}")), "{path}");
        assert!(
            !path.contains(&format!("{:.1} {:.1}", developer.centre().0, 400.0)),
            "nothing should be routed up the column centre: {path}"
        );
    }

    #[test]
    fn markup_in_a_name_cannot_escape_into_the_document() {
        // Names are validated, so this cannot happen today. It is guarded anyway because the
        // cost of being wrong is script injection into the operator's dashboard, and the cost of
        // the guard is one function.
        assert_eq!(
            escape(r#"<script>alert("x")</script>"#),
            "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"
        );
    }

    #[test]
    fn an_edge_to_a_node_that_is_not_there_is_skipped_rather_than_drawn_wrong() {
        let mut layout = Layout::build(&factory(), &Live::default());
        layout.edges.push(Edge {
            from: "a_analyst".to_owned(),
            to: "a_ghost".to_owned(),
            label: None,
            style: EdgeStyle::Plain,
            scopes: Vec::new(),
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
        });

        let svg = render(&layout);

        assert!(!svg.contains("a_ghost"));
    }
}
