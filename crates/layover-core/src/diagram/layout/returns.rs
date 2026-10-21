//! Where edges meet their nodes, and the lanes return paths run through.

use std::collections::BTreeMap;

use super::{
    COL_GAP, CORNER_MARGIN, EXIT_INSET, EXIT_STEP, GUTTER_INSET, GUTTER_MIN, GUTTER_STEP, Layout,
    NODE_W, RETURN_GAP, column_x, port, precise, stack,
};

/// How far into the gap left of a column a gutter must stay, so it never reaches the boxes of the
/// column before.
const GUTTER_CLEAR: f64 = 10.0;

impl Layout {
    /// Spreads each node's edges along its sides instead of bunching them at the centre.
    ///
    /// Ordered by where each edge goes next — the other end, or the first column it crosses on
    /// the way — so two edges leaving the same node never cross each other before they have gone
    /// anywhere. Only forward edges: a return path leaves from the underside and is routed
    /// through its own lane already.
    pub(super) fn assign_ports(&mut self) {
        let centres: BTreeMap<String, f64> = self
            .nodes
            .iter()
            .map(|node| (node.id.clone(), node.centre().1))
            .collect();
        let boxes: BTreeMap<String, (f64, f64)> = self
            .nodes
            .iter()
            .map(|node| (node.id.clone(), (node.y, node.h)))
            .collect();

        let mut leaving: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut arriving: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, edge) in self.edges.iter().enumerate() {
            if edge.back || edge.beside {
                continue;
            }
            leaving.entry(edge.from.clone()).or_default().push(index);
            arriving.entry(edge.to.clone()).or_default().push(index);
        }

        let next = |index: usize| {
            let edge = &self.edges[index];
            edge.via.first().map_or_else(
                || centres.get(&edge.to).copied().unwrap_or(0.0),
                |(_, y)| *y,
            )
        };
        let previous = |index: usize| {
            let edge = &self.edges[index];
            edge.via.last().map_or_else(
                || centres.get(&edge.from).copied().unwrap_or(0.0),
                |(_, y)| *y,
            )
        };

        let mut from_ys = Vec::new();
        for (id, mut indices) in leaving {
            indices.sort_by(|a, b| next(*a).total_cmp(&next(*b)));
            let Some(&(top, height)) = boxes.get(&id) else {
                continue;
            };
            let count = indices.len();
            for (slot, index) in indices.into_iter().enumerate() {
                from_ys.push((index, port(top, height, slot, count)));
            }
        }
        let mut to_ys = Vec::new();
        for (id, mut indices) in arriving {
            indices.sort_by(|a, b| previous(*a).total_cmp(&previous(*b)));
            let Some(&(top, height)) = boxes.get(&id) else {
                continue;
            };
            let count = indices.len();
            for (slot, index) in indices.into_iter().enumerate() {
                to_ys.push((index, port(top, height, slot, count)));
            }
        }
        for (index, y) in from_ys {
            self.edges[index].from_y = y;
        }
        for (index, y) in to_ys {
            self.edges[index].to_y = y;
        }
    }

    /// Gives each return path its own lane just below the columns it spans, and reports the
    /// deepest one and the rightmost point any of them drops at.
    ///
    /// Below the columns it spans rather than below the whole drawing: a loop from the fourth
    /// column back to the second has no business under the eighth, and the longest shapes on the
    /// map should not be the ones that mean least. Lanes rather than one shared depth: two loops
    /// over the same columns at the same height would be drawn on top of each other, so the
    /// shortest goes innermost and each one overlapping it runs below it.
    pub(super) fn route_returns(&mut self) -> (f64, f64) {
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

        let bottoms = self.column_bottoms();
        let boxes: BTreeMap<String, (f64, f64)> = self
            .nodes
            .iter()
            .map(|node| (node.id.clone(), (node.x, node.w)))
            .collect();

        // How many returns already aim at each target, so edges sharing one can be fanned apart
        // rather than stacked on a single vertical line.
        let mut per_target: BTreeMap<String, usize> = BTreeMap::new();
        // How many returns already leave each column, so their drops do not share a line either.
        let mut per_column: BTreeMap<usize, usize> = BTreeMap::new();
        let columns: BTreeMap<String, (usize, f64)> = self
            .nodes
            .iter()
            .map(|node| (node.id.clone(), (node.layer, node.x + node.w)))
            .collect();
        // Each lane placed so far: the columns it spans, and its depth.
        let mut placed: Vec<(usize, usize, f64)> = Vec::new();

        let mut deepest = 0.0_f64;
        let mut widest = 0.0_f64;
        for key in &lanes {
            let (layer, right) = columns.get(&key.0).copied().unwrap_or((0, 0.0));
            let target = columns.get(&key.1).map_or(0, |(layer, _)| *layer);
            let (low, high) = (layer.min(target), layer.max(target));

            // Below every box and crossing in the columns it spans, and below every lane already
            // placed whose climb or drop could share a gap with this one's.
            let lowest_box = bottoms
                .range(low..=high)
                .map(|(_, bottom)| *bottom)
                .fold(0.0_f64, f64::max);
            let lowest_lane = placed
                .iter()
                .filter(|(l, h, _)| *l <= high + 1 && low <= *h + 1)
                .map(|(_, _, depth)| *depth)
                .fold(0.0_f64, f64::max);
            let depth = lowest_box.max(lowest_lane) + RETURN_GAP;
            placed.push((low, high, depth));
            deepest = deepest.max(depth);

            let seen = per_target.entry(key.1.clone()).or_insert(0);
            let nth = *seen;
            *seen += 1;

            let (left, width) = boxes.get(&key.1).copied().unwrap_or((0.0, NODE_W));
            let gutter = (left - GUTTER_INSET - GUTTER_STEP * precise(nth))
                .max(left - COL_GAP + GUTTER_CLEAR)
                .max(GUTTER_MIN);
            let hook = left + width * 0.25 + width * 0.2 * precise(nth);

            let leaving = per_column.entry(layer).or_insert(0);
            // Capped short of the next column, whose boxes a drop must not run through.
            let exit = (right + EXIT_INSET + EXIT_STEP * precise(*leaving))
                .min(right + COL_GAP - EXIT_INSET / 2.0);
            *leaving += 1;
            widest = widest.max(exit);

            if let Some(edge) = self
                .edges
                .iter_mut()
                .find(|edge| edge.back && (edge.from.clone(), edge.to.clone()) == *key)
            {
                edge.floor = Some(depth);
                edge.gutter = Some(gutter);
                edge.hook_x = Some(hook.min(left + width - CORNER_MARGIN));
                edge.exit_x = Some(exit);
            }
        }
        (deepest, widest)
    }

    /// The lowest point of each column: its lowest box, or the lowest edge crossing it.
    fn column_bottoms(&self) -> BTreeMap<usize, f64> {
        let mut bottoms: BTreeMap<usize, f64> = BTreeMap::new();
        let mut lower = |column: usize, y: f64| {
            let bottom = bottoms.entry(column).or_insert(0.0);
            *bottom = bottom.max(y);
        };
        for node in &self.nodes {
            lower(node.layer, node.y + node.h);
        }
        for edge in &self.edges {
            for (x, y) in &edge.via {
                lower(column_of(*x), y + stack::THROUGH_H / 2.0);
            }
        }
        bottoms
    }
}

/// The column an x falls within: the last whose left edge is at or before it.
fn column_of(x: f64) -> usize {
    let mut column = 0;
    while column_x(column + 1) <= x {
        column += 1;
    }
    column
}
