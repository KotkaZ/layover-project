//! Which column each agent sits in: reading order, from the way in.
//!
//! A route map is read left to right, so an agent's column should say where it comes in the work,
//! not how few routes separate it from the entry. Those two differ as soon as a workflow has a
//! shortcut: an analyst allowed to send straight to the developer, the reviewer and the operator
//! for rare messages puts all three one hop from the entry, and breadth-first columns stacked them
//! in one column with the analyst's consults, which hid the order the work actually follows.
//!
//! So the columns come from a depth-first walk from the way in that visits each agent's
//! successors in the order `layover.toml` declares them — routes as listed, each route's `to` in
//! order. An edge into an agent still on the walk's stack closes a loop, and is set aside; every
//! other edge points forwards, and each agent's column is the longest path to it over those. The
//! edges set aside are drawn as what they are: routes back.
//!
//! Declaration order is therefore part of the picture, deliberately. A factory lists its routes in
//! roughly the order its work flows, and following that order is what lays a mesh with shortcuts
//! and loops out along its main path. Shuffle the routes and the map changes; the factory does not.

use std::collections::{BTreeMap, BTreeSet};

use crate::agent::AgentName;
use crate::config::Config;
use crate::diagram::Scope;

/// Assigns each agent a column, counting from zero at the first agent column.
///
/// Agents the walk never reaches are absent, and the caller puts them in the first column: an
/// unreachable agent is exactly what somebody opened the diagram to find.
///
/// One workflow is walked from its own entry over the routes its chains may use. A resuming
/// pipeline with no entry is walked from every pipeline's entry, so the agents it shares with them
/// sit where they do in theirs. The whole factory is walked from each pipeline's entry, by
/// pipeline name, then from each agent marked `entry = true`, by name: an agent two workflows
/// reach at different depths takes the deeper column.
///
/// A spawn is walked like any other route, which keeps a spawned agent right of its spawner —
/// unless the spawn closes a loop with other routes, as one workflow's spawn can with another's
/// routes on the whole-factory map. Then it is set aside like any other edge that does.
pub(super) fn assign(
    config: &Config,
    scope: &Scope,
    members: Option<&BTreeSet<AgentName>>,
) -> BTreeMap<AgentName, usize> {
    let next = successors(config, scope, members);
    let seeds = seeds(config, scope);

    let mut walk = Walk::default();
    for seed in &seeds {
        if members.is_none_or(|members| members.contains(seed)) {
            walk.from(seed, &next);
        }
    }

    // Reverse finishing order is a topological order of the edges kept, so one pass settles every
    // longest path.
    let mut layers: BTreeMap<AgentName, usize> =
        walk.finished.iter().map(|name| (name.clone(), 0)).collect();
    for from in walk.finished.iter().rev() {
        let here = layers[from];
        for to in next.get(from).into_iter().flatten() {
            if walk.back.contains(&(from.clone(), to.clone())) {
                continue;
            }
            if let Some(layer) = layers.get_mut(to) {
                *layer = (*layer).max(here + 1);
            }
        }
    }
    layers
}

/// Each agent's successors, in the order `layover.toml` declares them, each listed once.
fn successors(
    config: &Config,
    scope: &Scope,
    members: Option<&BTreeSet<AgentName>>,
) -> BTreeMap<AgentName, Vec<AgentName>> {
    let shown = |name: &AgentName| members.is_none_or(|members| members.contains(name));
    let mut next: BTreeMap<AgentName, Vec<AgentName>> = BTreeMap::new();

    for route in &config.routes {
        if scope
            .pipeline()
            .is_some_and(|name| !route.applies_to(Some(name)))
        {
            continue;
        }
        for from in route.from.iter().filter(|name| shown(name)) {
            for to in route.to.iter().filter(|name| shown(name)) {
                let listed = next.entry(from.clone()).or_default();
                if !listed.contains(to) {
                    listed.push(to.clone());
                }
            }
        }
    }
    next
}

/// Where the walk starts, in order.
fn seeds(config: &Config, scope: &Scope) -> Vec<AgentName> {
    let entries = || {
        config
            .pipelines
            .values()
            .filter_map(|pipeline| pipeline.entry.clone())
    };
    match scope.pipeline() {
        Some(name) => match config.pipelines.get(name).map(|pipeline| &pipeline.entry) {
            Some(Some(entry)) => vec![entry.clone()],
            Some(None) => entries().collect(),
            None => Vec::new(),
        },
        None => entries()
            .chain(
                config
                    .agents
                    .iter()
                    .filter(|(_, agent)| agent.entry)
                    .map(|(name, _)| name.clone()),
            )
            .collect(),
    }
}

/// A depth-first walk, remembering the order agents finish in and the edges that close a loop.
#[derive(Default)]
struct Walk {
    /// Agents on the current path.
    open: BTreeSet<AgentName>,
    /// Agents whose successors have all been walked, in the order that happened.
    finished: Vec<AgentName>,
    /// Edges into an agent that was on the path when they were walked.
    back: BTreeSet<(AgentName, AgentName)>,
}

impl Walk {
    /// Walks everything reachable from `start` that has not been walked yet.
    ///
    /// With an explicit stack rather than recursion, so a route map of any depth cannot exhaust
    /// the thread's stack.
    fn from(&mut self, start: &AgentName, next: &BTreeMap<AgentName, Vec<AgentName>>) {
        if self.seen(start) {
            return;
        }
        self.open.insert(start.clone());
        let mut stack: Vec<(AgentName, usize)> = vec![(start.clone(), 0)];

        while let Some((at, index)) = stack.last_mut() {
            let successor = next.get(at).and_then(|tos| tos.get(*index)).cloned();
            *index += 1;
            let at = at.clone();
            match successor {
                Some(to) if self.open.contains(&to) => {
                    self.back.insert((at, to));
                }
                Some(to) if self.seen(&to) => {}
                Some(to) => {
                    self.open.insert(to.clone());
                    stack.push((to, 0));
                }
                None => {
                    stack.pop();
                    self.open.remove(&at);
                    self.finished.push(at);
                }
            }
        }
    }

    fn seen(&self, name: &AgentName) -> bool {
        self.open.contains(name) || self.finished.contains(name)
    }
}
