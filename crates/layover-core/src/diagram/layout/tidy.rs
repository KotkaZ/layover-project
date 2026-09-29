//! Two passes that keep a busy route map legible.
//!
//! A real factory's routes mostly come in pairs: an analyst asks an investigator and hears back, a
//! builder hands a tester a change and gets a verdict. Each reverse direction used to be drawn as a
//! return path — a loop under the whole diagram, in a lane of its own — so a workflow of twenty
//! routes became eleven loops stacked hundreds of pixels deep, crossing everything on the way.
//!
//! [`pair_up`] draws a pair of plain routes as one line with an arrowhead at each end. That is not
//! a loss of information: the line says exactly "either may send to the other", and a join, a
//! spawn or a differing scope — anything that makes one direction mean something the other does
//! not — keeps its own arrow.
//!
//! [`route_beside`] draws a route between two agents in the same column beside that column rather
//! than under the whole drawing: a short straight connector between neighbours, and an arc that
//! swings out into the gap on the right for agents further apart.

use std::collections::BTreeMap;

use super::{COL_GAP, Edge, EdgeStyle, Node};

/// How far the innermost arc beside a column swings out.
const BULGE: f64 = 26.0;
/// How much further out an arc swings to clear one already there.
const BULGE_STEP: f64 = 14.0;
/// The furthest an arc may swing, which keeps it out of the next column.
const BULGE_MAX: f64 = COL_GAP - 20.0;
/// How far apart two connectors between the same neighbours run.
const CONNECTOR_SPREAD: f64 = 12.0;

/// Merges each pair of plain routes in opposite directions into one two-headed line.
///
/// The direction kept is the one that reads left to right, so the line is laid out as a forward
/// edge; for two agents in one column either will do.
pub(super) fn pair_up(edges: &mut Vec<Edge>) {
    let plain = |edge: &Edge| edge.style == EdgeStyle::Plain && edge.label.is_none();
    let mut dropped = vec![false; edges.len()];

    for i in 0..edges.len() {
        if dropped[i] || edges[i].both || !plain(&edges[i]) {
            continue;
        }
        let Some(j) = (i + 1..edges.len()).find(|&j| {
            !dropped[j]
                && plain(&edges[j])
                && edges[j].from == edges[i].to
                && edges[j].to == edges[i].from
                && edges[j].scopes == edges[i].scopes
        }) else {
            continue;
        };

        let (keep, drop) = if edges[i].back && !edges[j].back {
            (j, i)
        } else {
            (i, j)
        };
        edges[keep].both = true;
        dropped[drop] = true;
    }

    let mut index = 0;
    edges.retain(|_| {
        let keep = !dropped[index];
        index += 1;
        keep
    });
}

/// Routes every edge between two agents in the same column, and reports the rightmost x any of
/// them reaches, so the drawing can be made wide enough to hold it.
///
/// Neighbours get a straight connector through the gap between them, at the centre — or spread
/// apart when more than one edge joins the same two. Agents further apart get an arc out to the
/// right, shortest innermost, and an arc that overlaps one already placed swings further out
/// rather than crossing it.
pub(super) fn route_beside(nodes: &[Node], edges: &mut [Edge]) -> f64 {
    let find = |id: &str| nodes.iter().find(|node| node.id == id);
    let mut widest = 0.0_f64;

    let mut arcs = Vec::new();
    let mut connectors: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (index, edge) in edges.iter().enumerate() {
        if !edge.beside {
            continue;
        }
        let (Some(from), Some(to)) = (find(&edge.from), find(&edge.to)) else {
            continue;
        };
        let (top, bottom) = (from.y.min(to.y), from.y.max(to.y));
        let apart = nodes
            .iter()
            .any(|node| node.layer == from.layer && node.y > top && node.y < bottom);

        if apart {
            arcs.push((index, top, bottom, from.layer));
        } else {
            let mut ends = [edge.from.clone(), edge.to.clone()];
            ends.sort();
            let [first, second] = ends;
            connectors.entry((first, second)).or_default().push(index);
        }
    }

    for indices in connectors.into_values() {
        let count = indices.len();
        for (slot, index) in indices.into_iter().enumerate() {
            let Some(node) = find(&edges[index].from) else {
                continue;
            };
            let offset =
                (super::precise(slot) - super::precise(count - 1) / 2.0) * CONNECTOR_SPREAD;
            edges[index].gutter = Some(node.centre().0 + offset);
        }
    }

    arcs.sort_by(|a, b| (a.2 - a.1).total_cmp(&(b.2 - b.1)));
    let mut placed: BTreeMap<usize, Vec<(f64, f64, f64)>> = BTreeMap::new();
    for (index, top, bottom, layer) in arcs {
        let column = placed.entry(layer).or_default();
        let outermost = column
            .iter()
            .filter(|(t, b, _)| *t < bottom && *b > top)
            .map(|(_, _, bulge)| *bulge)
            .fold(0.0_f64, f64::max);
        let bulge = if outermost > 0.0 {
            outermost + BULGE_STEP
        } else {
            BULGE
        }
        .min(BULGE_MAX);
        column.push((top, bottom, bulge));

        edges[index].bulge = Some(bulge);
        if let Some(node) = find(&edges[index].from) {
            widest = widest.max(node.x + node.w + bulge);
        }
    }

    widest
}
