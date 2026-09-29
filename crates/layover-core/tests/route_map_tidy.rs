//! Keeping a busy route map legible.
//!
//! A real factory's routes mostly come in pairs — an analyst asks an investigator and hears back —
//! and every reverse direction used to be drawn as its own loop under the whole diagram. The
//! reference case below is a real `devforge` workflow: twenty routes, sixteen of them in reciprocal
//! pairs, drawn as eleven loops stacked four hundred pixels deep.

use layover_core::Config;
use layover_core::diagram::{Layout, Live, Scope, render_svg};

const DEVFORGE: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "copilot"

[runners.copilot]
command = ["copilot"]

[agents.analyst]
prompt = "a"
[agents.sherlock]
prompt = "s"
[agents.golddigger]
prompt = "g"
[agents.eagle]
prompt = "e"
[agents.bob]
prompt = "b"
[agents.wolf]
prompt = "w"
[agents.azurix]
prompt = "z"
[agents.mailman]
prompt = "m"

[pipelines.devforge]
entry = "analyst"

[[routes]]
from = "analyst"
to = ["sherlock", "golddigger", "eagle", "bob"]
[[routes]]
from = ["sherlock", "golddigger"]
to = ["analyst", "eagle"]
[[routes]]
from = "eagle"
to = ["analyst", "bob", "sherlock", "golddigger"]
[[routes]]
from = "bob"
to = ["wolf", "analyst", "azurix"]
[[routes]]
from = "wolf"
to = ["bob", "eagle", "azurix"]
[[routes]]
from = "azurix"
to = ["bob", "mailman"]
"#;

fn devforge() -> Layout {
    let config = Config::from_toml(DEVFORGE, "devforge.toml").expect("parses");
    Layout::scoped(
        &config,
        &Live::default(),
        &Scope::Pipeline("devforge".into()),
    )
}

fn between(layout: &Layout, a: &str, b: &str) -> usize {
    let (a, b) = (format!("a_{a}"), format!("a_{b}"));
    layout
        .edges
        .iter()
        .filter(|edge| (edge.from == a && edge.to == b) || (edge.from == b && edge.to == a))
        .count()
}

#[test]
fn a_route_both_ways_is_drawn_as_one_line() {
    let layout = devforge();

    for (a, b) in [
        ("analyst", "sherlock"),
        ("analyst", "golddigger"),
        ("analyst", "eagle"),
        ("analyst", "bob"),
        ("sherlock", "eagle"),
        ("golddigger", "eagle"),
        ("bob", "wolf"),
        ("bob", "azurix"),
    ] {
        assert_eq!(between(&layout, a, b), 1, "{a} and {b}");
    }
    assert_eq!(between(&layout, "azurix", "mailman"), 1);
}

#[test]
fn a_line_drawn_for_both_directions_has_an_arrowhead_at_each_end() {
    let svg = render_svg(&devforge());
    let two_headed = svg
        .lines()
        .filter(|line| line.contains("marker-start=") && line.contains("marker-end="))
        .count();

    assert_eq!(two_headed, 8, "{svg}");
}

#[test]
fn a_route_between_agents_in_one_column_is_not_a_loop_under_the_map() {
    let layout = devforge();
    let layer = |id: &str| layout.node(id).map(|node| node.layer);

    for edge in &layout.edges {
        if layer(&edge.from) == layer(&edge.to) {
            assert!(
                !edge.back,
                "{} -> {} shares a column and is drawn as a return",
                edge.from, edge.to
            );
        }
    }
}

#[test]
fn only_a_one_way_route_backwards_still_loops_under_the_map() {
    let layout = devforge();
    let returns: Vec<(&str, &str)> = layout
        .edges
        .iter()
        .filter(|edge| edge.back)
        .map(|edge| (edge.from.as_str(), edge.to.as_str()))
        .collect();

    assert_eq!(returns, [("a_wolf", "a_eagle")]);
}

#[test]
fn the_map_is_no_deeper_than_its_columns_and_one_lane() {
    let layout = devforge();
    let bottom = layout
        .nodes
        .iter()
        .map(|node| node.y + node.h)
        .fold(0.0_f64, f64::max);

    assert!(
        layout.height <= bottom + 80.0,
        "{} for columns ending at {bottom}",
        layout.height
    );
}

#[test]
fn an_arc_beside_a_column_stays_inside_the_drawing_and_out_of_the_next_column() {
    let layout = devforge();

    for edge in layout.edges.iter().filter(|edge| edge.beside) {
        let node = layout.node(&edge.from).expect("drawn");
        if let Some(bulge) = edge.bulge {
            let reach = node.x + node.w + bulge;
            assert!(reach <= layout.width, "{reach} > {}", layout.width);
            assert!(
                bulge < 96.0,
                "{} -> {} bulges into the next column",
                edge.from,
                edge.to
            );
        }
    }
    assert!(
        layout.edges.iter().any(|edge| edge.beside),
        "the fixture has routes within a column"
    );
}

#[test]
fn a_return_path_runs_along_its_lane_rather_than_curving_short_of_it() {
    // One cubic curve with its control points on the lane never reaches the lane: its lowest
    // point sits a quarter of the way short. With a single return path the lane is only just
    // below the deepest box, so the curve ran behind the box it was routed around.
    let layout = devforge();
    let back = layout
        .edges
        .iter()
        .find(|edge| edge.back)
        .expect("the fixture has a return path");
    let floor = back.floor.expect("a lane");

    let svg = render_svg(&layout);
    let path = svg
        .lines()
        .filter_map(|line| line.split(r#" d=""#).nth(1)?.split('"').next())
        .find(|d| d.contains(" Q "))
        .expect("the return path is drawn");

    let tokens: Vec<&str> = path.split_whitespace().collect();
    let runs_along = tokens.windows(3).any(|window| {
        window[0] == "L"
            && window[2]
                .parse::<f64>()
                .is_ok_and(|y| (y - floor).abs() < 0.5)
    });
    assert!(
        runs_along,
        "no straight run at the lane's depth {floor}: {path}"
    );
}

/// The straight runs of a path, as `((x0, y0), (x1, y1))`, from the `M`, `L` and `Q` commands this
/// renderer writes.
fn straight_runs(path: &str) -> Vec<((f64, f64), (f64, f64))> {
    let tokens: Vec<&str> = path.split_whitespace().collect();
    let number = |i: usize| tokens[i].parse::<f64>().expect("a coordinate");
    let (mut at, mut i, mut runs) = ((0.0, 0.0), 0, Vec::new());
    while i < tokens.len() {
        match tokens[i] {
            "M" => {
                at = (number(i + 1), number(i + 2));
                i += 3;
            }
            "L" => {
                let to = (number(i + 1), number(i + 2));
                runs.push((at, to));
                at = to;
                i += 3;
            }
            "Q" => {
                at = (number(i + 3), number(i + 4));
                i += 5;
            }
            "C" => {
                at = (number(i + 5), number(i + 6));
                i += 7;
            }
            other => panic!("unexpected {other} in {path}"),
        }
    }
    runs
}

#[test]
fn no_return_path_runs_through_a_box() {
    // A return path used to leave from the bottom of its box and drop straight down through every
    // box below it in the same column. Between the boxes it showed as a short vertical line, which
    // reads as a route between neighbours that does not exist.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/workitem-factory/layover.toml"
    );
    let config = Config::load(path).expect("the reference factory loads");
    for layout in [Layout::build(&config, &Live::default()), devforge()] {
        let svg = render_svg(&layout);
        let returns: Vec<&str> = svg
            .lines()
            .filter_map(|line| line.split(r#" d=""#).nth(1)?.split('"').next())
            .filter(|d| d.contains(" Q "))
            .collect();
        assert!(!returns.is_empty(), "there are return paths to check");

        for d in returns {
            for ((x0, y0), (x1, y1)) in straight_runs(d) {
                let (left, right) = (x0.min(x1), x0.max(x1));
                let (top, bottom) = (y0.min(y1), y0.max(y1));
                for node in &layout.nodes {
                    let crosses = left < node.x + node.w - 1.0
                        && right > node.x + 1.0
                        && top < node.y + node.h - 1.0
                        && bottom > node.y + 1.0;
                    assert!(!crosses, "{d} runs through {}", node.id);
                }
            }
        }
    }
}

const REVIEW_LOOP: &str = r#"
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

[pipelines.go]
entry = "dev"

[[routes]]
from = "dev"
to = "tester"

[[routes]]
from = "tester"
to = "dev"
join = "all"
"#;

#[test]
fn a_join_keeps_its_own_labelled_arrow_back() {
    // Merging would hide the barrier: the way back parks at it and the way out does not.
    let config = Config::from_toml(REVIEW_LOOP, "loop.toml").expect("parses");
    let layout = Layout::build(&config, &Live::default());

    assert_eq!(between(&layout, "dev", "tester"), 2);
    assert!(
        layout
            .edges
            .iter()
            .any(|edge| edge.back && edge.label.as_deref() == Some("all"))
    );
}

#[test]
fn two_directions_scoped_to_different_workflows_stay_two_lines_on_the_whole_map() {
    let text = r#"
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

[pipelines.go]
entry = "dev"

[pipelines.other]
entry = "tester"

[[routes]]
from = "dev"
to = "tester"
pipelines = "go"

[[routes]]
from = "tester"
to = "dev"
pipelines = "other"
"#;
    let config = Config::from_toml(text, "scoped.toml").expect("parses");
    let layout = Layout::build(&config, &Live::default());

    assert_eq!(
        between(&layout, "dev", "tester"),
        2,
        "one line cannot say two different scopes"
    );
}
