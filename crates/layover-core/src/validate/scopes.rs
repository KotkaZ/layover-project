//! Checks about route scopes: that they name real pipelines, that routes sharing a scope agree,
//! and that a scoped route is one some chain can actually use.
//!
//! None of these can fire on a factory that scopes nothing. Every existing factory keeps exactly
//! the findings it had, which is why a disagreement between two *global* routes is still left to
//! the checks that existed before scopes did.

use std::collections::{BTreeMap, BTreeSet};

use crate::agent::AgentName;
use crate::config::Config;
use crate::pipeline::PipelineName;
use crate::route::Route;
use crate::scope::RouteMap;

use super::Diagnostic;

pub(super) fn check_route_scopes(config: &Config, found: &mut Vec<Diagnostic>) {
    check_scopes_name_pipelines(config, found);
    check_overlapping_routes_agree(config, found);
    check_scoped_routes_can_be_used(config, found);
    check_direct_entries_are_not_stranded(config, found);
}

/// A scope names declared pipelines, and at least one of them.
///
/// An empty list is refused rather than read either way: as "no pipelines" it is a route nothing
/// may use, and as "every pipeline" it is the widest permission in the factory. A typo in a
/// pipeline name is refused for the same reason a typo in an agent name is — the route would
/// silently apply to nothing, and the workflow it was written for would be missing an edge.
fn check_scopes_name_pipelines(config: &Config, found: &mut Vec<Diagnostic>) {
    for (index, route) in config.routes.iter().enumerate() {
        let Some(names) = &route.pipelines else {
            continue;
        };

        if names.is_empty() {
            found.push(Diagnostic::error(format!(
                "route {index} has `pipelines = []`, which names no pipeline; leave `pipelines` \
                 out for a route every chain may use"
            )));
        }

        for name in names {
            if !config.pipelines.contains_key(name) {
                found.push(Diagnostic::error(format!(
                    "route {index} is scoped to unknown pipeline `{name}`"
                )));
            }
        }
    }
}

/// Two routes some chain could use together must agree about each edge they share.
///
/// A route's `mode` and `join` are properties of the route *in its scope*, and two routes naming
/// the same pair for overlapping pipelines would give one chain two answers: whether a send
/// continues the chain or opens a new one, and whether a spawned flight also waits at a barrier it
/// can never fill. Only pairs involving a scoped route are checked here; the same ambiguity
/// between two global routes predates scopes and is left as it was.
fn check_overlapping_routes_agree(config: &Config, found: &mut Vec<Diagnostic>) {
    let routes = &config.routes;

    for (i, first) in routes.iter().enumerate() {
        for (j, second) in routes.iter().enumerate().skip(i + 1) {
            if !(first.is_scoped() || second.is_scoped()) || !first.overlaps(second) {
                continue;
            }
            let place = where_both(first, second);

            if first.is_spawn() != second.is_spawn()
                && let Some((from, to)) = shared_pairs(first, second).into_iter().next()
            {
                found.push(Diagnostic::error(format!(
                    "routes {i} and {j} both permit `{from}` -> `{to}` {place}, but only one of \
                     them spawns; a flight on that edge cannot both continue its chain and open a \
                     new one"
                )));
            }

            for (spawn, join, spawn_at, join_at) in [(first, second, i, j), (second, first, j, i)] {
                if !spawn.is_spawn() || !join.is_join() {
                    continue;
                }
                for to in &join.to {
                    let Some(from) = join
                        .from
                        .iter()
                        .find(|from| spawn.from.contains(from) && spawn.to.contains(to))
                    else {
                        continue;
                    };
                    found.push(Diagnostic::error(format!(
                        "route {spawn_at} spawns from `{from}` to `{to}` {place}, where route \
                         {join_at} makes `{to}` wait for `{from}` at a barrier; a spawned flight \
                         opens a chain of its own and would reach the barrier alone and park there \
                         forever"
                    )));
                }
            }
        }
    }
}

/// Every sender/receiver pair both routes name.
fn shared_pairs<'a>(first: &'a Route, second: &Route) -> Vec<(&'a AgentName, &'a AgentName)> {
    first
        .from
        .iter()
        .filter(|from| second.from.contains(from))
        .flat_map(|from| {
            first
                .to
                .iter()
                .filter(|to| second.to.contains(to))
                .map(move |to| (from, to))
        })
        .collect()
}

/// Says where two overlapping routes collide, for a finding.
pub(super) fn where_both(first: &Route, second: &Route) -> String {
    let shared = first.shared_pipelines(second);
    if shared.is_empty() {
        return "in every chain".to_owned();
    }
    let names = shared
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("in pipeline(s) {names}")
}

/// Warns about a scoped route no chain of its pipelines can use.
///
/// A scoped route is used only by chains belonging to one of its pipelines, so it is dead when
/// none of those chains can ever wake its sender. It is not an error — the factory still runs —
/// but it is a permission somebody wrote for a workflow that does not work the way they thought.
///
/// What a pipeline's chains can wake:
///
/// - from its entry, across ordinary and spawn edges, because a spawned chain inherits the
///   pipeline;
/// - for a pipeline that `resumes`, also from **any agent some chain can wake**. Resumed work goes
///   back to whichever agent booked the layover, not to `entry`, and any agent may book one.
///   Counting all of them keeps this check from warning about a route that a follow-up really
///   does use.
fn check_scoped_routes_can_be_used(config: &Config, found: &mut Vec<Diagnostic>) {
    if !config.routes.iter().any(Route::is_scoped) {
        return;
    }

    let routes = RouteMap::from_config(config);
    let reach = what_each_pipeline_can_wake(config, &routes);

    for (index, route) in config.routes.iter().enumerate() {
        let Some(names) = &route.pipelines else {
            continue;
        };
        let known: Vec<&PipelineName> = names
            .iter()
            .filter(|name| reach.contains_key(name))
            .collect();
        if known.is_empty() {
            continue;
        }

        for from in &route.from {
            if known.iter().any(|name| reach[name].contains(from)) {
                continue;
            }
            let pipelines = known
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ");
            found.push(Diagnostic::warning(format!(
                "route {index} is scoped to {pipelines}, but no chain of that scope can wake \
                 `{from}`, so nothing can use it"
            )));
        }
    }
}

/// Every agent a chain of each pipeline can wake.
fn what_each_pipeline_can_wake<'a>(
    config: &'a Config,
    routes: &RouteMap,
) -> BTreeMap<&'a PipelineName, BTreeSet<AgentName>> {
    let direct = config
        .agents
        .iter()
        .filter(|(_, agent)| agent.entry)
        .map(|(name, _)| name);
    let mut woken = routes.for_pipeline(None).workflow_from_all(direct);

    for (name, pipeline) in &config.pipelines {
        if !pipeline.resumes {
            woken.extend(
                routes
                    .for_pipeline(Some(name))
                    .workflow_from(&pipeline.entry),
            );
        }
    }

    // A resumed chain can book a layover too, so what resuming pipelines wake feeds back into
    // where they can start. Each pass only adds, so this settles within one pass per agent.
    loop {
        let before = woken.len();
        for (name, pipeline) in config.pipelines.iter().filter(|(_, p)| p.resumes) {
            let starts: Vec<AgentName> = woken
                .iter()
                .cloned()
                .chain(std::iter::once(pipeline.entry.clone()))
                .collect();
            woken.extend(routes.for_pipeline(Some(name)).workflow_from_all(&starts));
        }
        if woken.len() == before {
            break;
        }
    }

    config
        .pipelines
        .iter()
        .map(|(name, pipeline)| {
            let graph = routes.for_pipeline(Some(name));
            let reach = if pipeline.resumes {
                graph.workflow_from_all(woken.iter().chain(std::iter::once(&pipeline.entry)))
            } else {
                graph.workflow_from(&pipeline.entry)
            };
            (name, reach)
        })
        .collect()
}

/// Warns when a direct trigger would leave an `entry = true` agent with nowhere to send.
///
/// A flight sent straight to an agent, rather than through a pipeline, belongs to no pipeline and
/// may use global routes only. An entry agent whose every outgoing route is scoped can therefore
/// be woken that way and then reach nobody — the work stops at the first agent, looking finished.
fn check_direct_entries_are_not_stranded(config: &Config, found: &mut Vec<Diagnostic>) {
    for (name, agent) in &config.agents {
        if !agent.entry {
            continue;
        }

        let mut outgoing = config
            .routes
            .iter()
            .filter(|route| route.from.contains(name))
            .peekable();
        if outgoing.peek().is_none() {
            continue;
        }

        if outgoing.all(Route::is_scoped) {
            found.push(Diagnostic::warning(format!(
                "agent `{name}` is marked `entry = true`, but every route out of it is scoped to \
                 a pipeline. A flight sent straight to it belongs to no pipeline and may use \
                 global routes only, so it could send nothing; trigger it through a pipeline, or \
                 leave a route out of it unscoped"
            )));
        }
    }
}

#[cfg(test)]
mod tests;
