//! Ordering each column, and stacking it.
//!
//! Columns follow reading order, so an edge may skip several: the analyst may hand straight to an
//! operator four columns on. Drawn as one curve, such an edge ran through whatever boxes stood in
//! between, and a line through a box reads as a route to it. So an edge that skips columns is
//! given a point in every column it crosses — a thin row of its own, ordered and stacked with the
//! boxes like any other — and is drawn through the gaps those rows leave.
//!
//! Each column is ordered by the barycentre of what feeds it from the column before: one sweep,
//! left to right, so a column is ordered against the column before it as that column will be
//! drawn. It is not optimal — crossing minimisation is NP-hard — but on a graph this size one
//! sweep removes the tangles a reader would notice.

use std::collections::BTreeMap;

use super::{Layout, MARGIN, NODE_W, ROW_GAP, column_x, precise};

/// Height of the row an edge crossing a column takes up there.
pub(super) const THROUGH_H: f64 = 4.0;
/// Gap kept between such a row and whatever is above or below it.
const THROUGH_GAP: f64 = 14.0;

/// One row of a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Row {
    /// A node's box, by its index in [`Layout::nodes`].
    Node(usize),
    /// The point where an edge, by its index in [`Layout::edges`], crosses this column.
    Through(usize),
}

impl Layout {
    /// Orders every column, stacks it centred on a common middle line, and records where each edge
    /// that skips columns crosses them.
    pub(super) fn stack(&mut self) {
        let height = self.nodes.first().map_or(0.0, |node| node.h);
        let mut columns = self.columns();
        let mut middles: BTreeMap<(usize, Row), f64> = BTreeMap::new();

        for (column, rows) in &mut columns {
            let at = centred(rows, height);
            let fed: Vec<Option<f64>> = rows
                .iter()
                .map(|row| self.fed_from(*column, *row, &middles))
                .collect();

            let mut order: Vec<usize> = (0..rows.len()).collect();
            order.sort_by(|&a, &b| {
                let key = |index: usize| fed[index].unwrap_or(at[index]);
                key(a)
                    .total_cmp(&key(b))
                    .then_with(|| self.tie(rows[a]).cmp(&self.tie(rows[b])))
            });
            *rows = order.into_iter().map(|index| rows[index]).collect();

            for (row, middle) in rows.iter().zip(centred(rows, height)) {
                middles.insert((*column, *row), middle);
            }
        }

        let top = middles
            .iter()
            .map(|(&(_, row), middle)| middle - row_height(row, height) / 2.0)
            .fold(f64::INFINITY, f64::min);
        let shift = if top.is_finite() { MARGIN - top } else { 0.0 };

        for edge in &mut self.edges {
            edge.via.clear();
        }
        for (&(column, row), middle) in &middles {
            let y = middle + shift;
            match row {
                Row::Node(index) => self.nodes[index].y = y - self.nodes[index].h / 2.0,
                Row::Through(index) => {
                    let x = column_x(column);
                    self.edges[index].via.push((x, y));
                    self.edges[index].via.push((x + NODE_W, y));
                }
            }
        }
        for edge in &mut self.edges {
            let mut pairs: Vec<[(f64, f64); 2]> =
                edge.via.chunks(2).map(|pair| [pair[0], pair[1]]).collect();
            pairs.sort_by(|a, b| a[0].0.total_cmp(&b[0].0));
            edge.via = pairs.into_iter().flatten().collect();
        }
    }

    /// Every column's rows, nodes first in the order they were placed, then a row for each edge
    /// that crosses it.
    fn columns(&self) -> BTreeMap<usize, Vec<Row>> {
        let mut columns: BTreeMap<usize, Vec<Row>> = BTreeMap::new();
        for (index, node) in self.nodes.iter().enumerate() {
            columns
                .entry(node.layer)
                .or_default()
                .push(Row::Node(index));
        }
        for (index, edge) in self.edges.iter().enumerate() {
            if edge.back || edge.beside {
                continue;
            }
            let (from, to) = (self.layer_of(&edge.from), self.layer_of(&edge.to));
            for column in from + 1..to {
                columns.entry(column).or_default().push(Row::Through(index));
            }
        }
        columns
    }

    /// Where a row is fed from in the column before: the mean middle of whatever arrives at it
    /// from there, or `None` when nothing does and it keeps its place.
    fn fed_from(
        &self,
        column: usize,
        row: Row,
        middles: &BTreeMap<(usize, Row), f64>,
    ) -> Option<f64> {
        let before = column.checked_sub(1)?;
        let arriving: Vec<usize> = match row {
            Row::Through(index) => vec![index],
            Row::Node(index) => self
                .edges
                .iter()
                .enumerate()
                .filter(|(_, edge)| edge.to == self.nodes[index].id && !edge.back && !edge.beside)
                .map(|(edge, _)| edge)
                .collect(),
        };
        let found: Vec<f64> = arriving
            .into_iter()
            .filter_map(|edge| {
                let from = &self.edges[edge].from;
                if self.layer_of(from) == before {
                    let node = self.nodes.iter().position(|node| node.id == *from)?;
                    middles.get(&(before, Row::Node(node))).copied()
                } else {
                    middles.get(&(before, Row::Through(edge))).copied()
                }
            })
            .collect();
        (!found.is_empty()).then(|| found.iter().sum::<f64>() / precise(found.len()))
    }

    /// What breaks a tie between two rows fed from the same place: boxes before crossings, then
    /// by name, so the order never depends on anything a reader cannot see.
    fn tie(&self, row: Row) -> (u8, String) {
        match row {
            Row::Node(index) => (0, self.nodes[index].label.clone()),
            Row::Through(index) => (
                1,
                format!("{}>{}", self.edges[index].from, self.edges[index].to),
            ),
        }
    }
}

/// The middle of each row when `rows` is stacked in this order, centred on zero.
fn centred(rows: &[Row], height: f64) -> Vec<f64> {
    let mut middles = Vec::with_capacity(rows.len());
    let mut y = 0.0;
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            y += gap(rows[index - 1], *row);
        }
        let h = row_height(*row, height);
        middles.push(y + h / 2.0);
        y += h;
    }
    middles.iter().map(|middle| middle - y / 2.0).collect()
}

/// How tall a row is: a box is as tall as every box in the drawing, a crossing is a thin line.
fn row_height(row: Row, height: f64) -> f64 {
    match row {
        Row::Node(_) => height,
        Row::Through(_) => THROUGH_H,
    }
}

/// The space kept between two rows: a full gap between boxes, and less beside a crossing, whose
/// line needs only enough to stay clear of the boxes either side.
fn gap(above: Row, below: Row) -> f64 {
    match (above, below) {
        (Row::Node(_), Row::Node(_)) => ROW_GAP,
        _ => THROUGH_GAP,
    }
}
