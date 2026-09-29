//! What the route map gives a page that wants to trace one agent's routes.
//!
//! The dashboard lights an agent's routes and neighbours when it is hovered, and pins them when
//! it is clicked. It needs nothing but the drawing to do that: every route says which two boxes it
//! joins, says what it is when hovered, and is wide enough to hover at all.

use layover_core::Config;
use layover_core::diagram::{Layout, Live, render_svg};

const FACTORY: &str = r#"
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
[agents.azurix]
prompt = "z"
[agents.eagle]
prompt = "e"

[pipelines.devforge]
entry = "analyst"

[pipelines.eagle-eye]
entry = "azurix"

[[routes]]
from = "analyst"
to = "sherlock"

[[routes]]
from = "sherlock"
to = "analyst"

[[routes]]
from = "azurix"
to = "eagle"
mode = "spawn"
pipelines = "eagle-eye"
"#;

fn drawn() -> (Layout, String) {
    let config = Config::from_toml(FACTORY, "trace.toml").expect("parses");
    let layout = Layout::build(&config, &Live::default());
    let svg = render_svg(&layout);
    (layout, svg)
}

/// The markup of the route joining `from` to `to`.
fn link<'a>(svg: &'a str, from: &str, to: &str) -> &'a str {
    let opening = format!(r#"data-from="{from}" data-to="{to}""#);
    let start = svg
        .find(&opening)
        .unwrap_or_else(|| panic!("no route {from} -> {to} in {svg}"));
    let rest = &svg[start..];
    &rest[..rest.find("</g>").expect("closed")]
}

#[test]
fn every_route_names_the_two_boxes_it_joins() {
    let (layout, svg) = drawn();

    for edge in &layout.edges {
        assert!(
            svg.contains(&format!(
                r#"data-from="{}" data-to="{}""#,
                edge.from, edge.to
            )),
            "{} -> {} does not say what it joins",
            edge.from,
            edge.to
        );
    }
}

#[test]
fn a_route_says_what_it_is_when_hovered() {
    let (layout, svg) = drawn();

    let pair = layout
        .edges
        .iter()
        .find(|edge| edge.both)
        .expect("the reciprocal pair is one line");
    let (from, to) = (&pair.from[2..], &pair.to[2..]);
    assert!(
        link(&svg, &pair.from, &pair.to).contains(&format!("<title>{from} ⇄ {to}</title>")),
        "{svg}"
    );
    assert!(
        svg.contains(&format!(
            r#"<g class="route both" data-from="{}" data-to="{}">"#,
            pair.from, pair.to
        )),
        "a page tracing an agent needs to know the line runs both ways: {svg}"
    );

    let spawn = link(&svg, "a_azurix", "a_eagle");
    assert!(
        spawn
            .contains("<title>azurix → eagle · spawns a new itinerary · only in eagle-eye</title>"),
        "{spawn}"
    );

    let entry = link(&svg, "p_devforge", "a_analyst");
    assert!(
        entry.contains("<title>devforge → analyst · way in</title>"),
        "{entry}"
    );
}

#[test]
fn a_thin_route_is_wide_enough_to_hover() {
    let (layout, svg) = drawn();

    for edge in &layout.edges {
        assert!(
            link(&svg, &edge.from, &edge.to).contains(r#"class="hit""#),
            "{} -> {} has nothing wide enough to point at",
            edge.from,
            edge.to
        );
    }
}
