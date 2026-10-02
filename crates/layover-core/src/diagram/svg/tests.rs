//! The SVG renderer's tests, kept apart so the renderer itself can be read whole.

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
