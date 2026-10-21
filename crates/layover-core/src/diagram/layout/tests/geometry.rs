//! No line passes through a box it does not connect.
//!
//! A line through a box reads as a route to it, so this is the bar every route on a map is held
//! to: forward edges, edges that skip columns, return paths and edges beside a column alike. It
//! samples the paths exactly as they are drawn, from the SVG, rather than the layout's intent.

use super::reading::{devforge, production};
use super::*;
use crate::diagram::render_svg;

/// A route as drawn: the identifiers of its two ends, and points its path passes through.
type Drawn = (String, String, Vec<(f64, f64)>);

/// Each route drawn.
fn routes(svg: &str) -> Vec<Drawn> {
    let mut found = Vec::new();
    for group in svg.split("<g class=\"route").skip(1) {
        let attribute = |name: &str| {
            let start = group
                .find(&format!("{name}=\""))
                .map(|at| at + name.len() + 2)?;
            let end = group[start..].find('"')? + start;
            Some(group[start..end].to_owned())
        };
        let (Some(from), Some(to), Some(path)) = (
            attribute("data-from"),
            attribute("data-to"),
            attribute(" d"),
        ) else {
            continue;
        };
        found.push((from, to, sample(&path)));
    }
    found
}

/// Points along an SVG path of absolute `M`, `L`, `C` and `Q` commands.
fn sample(path: &str) -> Vec<(f64, f64)> {
    let tokens: Vec<&str> = path.split_whitespace().collect();
    let number = |at: usize| tokens[at].parse::<f64>().expect("a coordinate");
    let mut points = Vec::new();
    let mut at = (0.0, 0.0);
    let mut index = 0;
    while index < tokens.len() {
        match tokens[index] {
            "M" => {
                at = (number(index + 1), number(index + 2));
                index += 3;
            }
            "L" => {
                let to = (number(index + 1), number(index + 2));
                points
                    .extend(steps().map(|t| (at.0 + (to.0 - at.0) * t, at.1 + (to.1 - at.1) * t)));
                at = to;
                index += 3;
            }
            "Q" => {
                let (c, to) = (
                    (number(index + 1), number(index + 2)),
                    (number(index + 3), number(index + 4)),
                );
                points.extend(steps().map(|t| {
                    let u = 1.0 - t;
                    (
                        u * u * at.0 + 2.0 * u * t * c.0 + t * t * to.0,
                        u * u * at.1 + 2.0 * u * t * c.1 + t * t * to.1,
                    )
                }));
                at = to;
                index += 5;
            }
            "C" => {
                let c1 = (number(index + 1), number(index + 2));
                let c2 = (number(index + 3), number(index + 4));
                let to = (number(index + 5), number(index + 6));
                points.extend(steps().map(|t| {
                    let rest = 1.0 - t;
                    let weights = [
                        rest * rest * rest,
                        3.0 * rest * rest * t,
                        3.0 * rest * t * t,
                        t * t * t,
                    ];
                    let mix = |coordinates: [f64; 4]| {
                        coordinates
                            .iter()
                            .zip(weights)
                            .map(|(value, weight)| value * weight)
                            .sum::<f64>()
                    };
                    (mix([at.0, c1.0, c2.0, to.0]), mix([at.1, c1.1, c2.1, to.1]))
                }));
                at = to;
                index += 7;
            }
            other => panic!("unexpected path command {other} in {path}"),
        }
    }
    points
}

fn steps() -> impl Iterator<Item = f64> {
    (0..=48).map(|step| f64::from(step) / 48.0)
}

/// Every point where a route passes inside a box it does not connect.
fn crossings(layout: &Layout) -> Vec<String> {
    let mut found = Vec::new();
    for (from, to, points) in routes(&render_svg(layout)) {
        for node in &layout.nodes {
            if node.id == from || node.id == to {
                continue;
            }
            let inside = |(x, y): &(f64, f64)| {
                *x > node.x + 0.5
                    && *x < node.x + node.w - 0.5
                    && *y > node.y + 0.5
                    && *y < node.y + node.h - 0.5
            };
            if let Some(point) = points.iter().find(|point| inside(point)) {
                found.push(format!(
                    "{from} -> {to} passes through {} at {point:?}",
                    node.id
                ));
            }
        }
    }
    found
}

#[test]
fn no_route_passes_through_a_box_it_does_not_connect() {
    let production = production();
    let mut maps = vec![
        ("devforge", devforge(&production)),
        (
            "whole factory",
            Layout::build(&production, &Live::default()),
        ),
        ("reference", Layout::build(&factory(), &Live::default())),
        (
            "review loop",
            Layout::build(&review_loop(), &Live::default()),
        ),
        (
            "two workflows",
            Layout::build(&two_workflows(), &Live::default()),
        ),
    ];
    for pipeline in ["eagle-eye", "chromium-webrtc-monitor"] {
        maps.push((
            pipeline,
            Layout::scoped(
                &production,
                &Live::default(),
                &Scope::Pipeline(pipeline.into()),
            ),
        ));
    }

    for (name, layout) in maps {
        let found = crossings(&layout);
        assert!(found.is_empty(), "{name}: {found:#?}");
    }
}

#[test]
fn the_check_sees_a_line_drawn_through_a_box() {
    // So that the test above passing means something: the same check, on a map where a route is
    // deliberately pushed through the box between its ends.
    let mut layout = devforge(&production());
    let developer = layout.node("a_developer").expect("developer").clone();
    let operator = layout
        .edges
        .iter_mut()
        .find(|edge| edge.from == "a_analyst" && edge.to == "a_ado-operator")
        .expect("the analyst's route to the operator");
    operator.via = vec![
        (developer.x, developer.centre().1),
        (developer.x + developer.w, developer.centre().1),
    ];

    assert!(!crossings(&layout).is_empty());
}
