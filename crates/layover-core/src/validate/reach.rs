//! Checks about what an itinerary can actually reach: entry points and hop depth.

use std::collections::BTreeMap;

use crate::agent::AgentName;
use crate::config::Config;
use crate::graph::RouteGraph;
use crate::route::Join;

use super::Diagnostic;

pub(super) fn check_entry_points(config: &Config, found: &mut Vec<Diagnostic>) {
    if config.agents.is_empty() {
        found.push(Diagnostic::error("the factory defines no agents"));
        return;
    }

    if config.entry_agents().next().is_none() {
        found.push(Diagnostic::error(
            "no agent is marked `entry = true` and no pipeline declares one, so nothing can be \
             triggered",
        ));
    }
}

/// Warns about agents an itinerary can never wake.
///
/// Two findings share one breadth-first walk, because "no edge leads here" and "Hops runs out
/// before arriving" are the same failure from an operator's point of view: the agent is
/// configured, looks live in the route map, and never runs.
///
/// The hop finding measures *shortest* paths, so it proves an agent is out of reach and never
/// proves one is in reach. A factory whose agents loop spends far more hops than the shortest
/// path suggests, and no static check can know how many times a loop will turn.
pub(super) fn check_every_agent_is_within_reach(config: &Config, found: &mut Vec<Diagnostic>) {
    let graph = RouteGraph::from_config(config);

    // Spawn targets count as entry points here. Each begins a fresh itinerary with a full hop
    // budget, so measuring its depth from the trigger that eventually caused it would warn that a
    // chain is cut when that chain has not even started yet.
    let mut entries: Vec<&AgentName> = config.entry_agents().collect();
    entries.extend(graph.spawn_targets());
    entries.sort();
    entries.dedup();
    if entries.is_empty() {
        return;
    }

    let distances = graph.distances_from(entries);
    let max_hops = config.defaults.max_hops;

    for name in config.agents.keys() {
        let Some(&distance) = distances.get(name) else {
            found.push(Diagnostic::warning(format!(
                "agent `{name}` cannot be reached from any entry point"
            )));
            continue;
        };

        // An agent at distance `d` is woken by flight `d + 1`, and a chain carries at most
        // `max_hops` flights.
        let flights = distance + 1;
        if flights > max_hops {
            found.push(Diagnostic::warning(format!(
                "agent `{name}` is {flights} flights from the nearest entry point but `max_hops` \
                 is {max_hops}, so the chain is cut before it arrives"
            )));
        }
    }

    check_joins_can_be_satisfied(config, &graph, &distances, max_hops, found);
}

/// Warns about a barrier no chain is long enough to fill.
///
/// The reach check above is not enough on its own. It takes the *shortest* path into an agent, but
/// a `join = "all"` target does not wake when the nearest upstream arrives — it waits for every
/// one of them. An upstream at distance `d` is woken by flight `d + 1` and must then spend flight
/// `d + 2` delivering to the barrier, so if any upstream cannot afford that flight, the barrier can
/// never release and the itinerary stalls rather than failing.
///
/// This uses shortest distances, so it only ever reports a barrier that is impossible even in the
/// best case. It will not catch every under-sized budget — nothing static can, once loops are
/// involved — but what it does report is always real.
fn check_joins_can_be_satisfied(
    config: &Config,
    graph: &RouteGraph,
    distances: &BTreeMap<AgentName, u32>,
    max_hops: u32,
    found: &mut Vec<Diagnostic>,
) {
    for target in config.agents.keys() {
        let Some(spec) = graph.join_for(target) else {
            continue;
        };
        if spec.join != Join::All {
            continue;
        }

        for upstream in &spec.upstreams {
            let Some(&distance) = distances.get(upstream) else {
                continue;
            };

            let delivery = distance + 2;
            if delivery > max_hops {
                found.push(Diagnostic::warning(format!(
                    "agent `{target}` waits for every upstream, but `{upstream}` could only \
                     deliver on flight {delivery} and `max_hops` is {max_hops}; the barrier can \
                     never release and the itinerary would stall"
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::validate::testing::{assert_mentions, errors, parse, warnings};
    use crate::validate::{has_errors, validate};

    /// A four-agent line, so the last agent needs four flights to be woken.
    fn line_of_four(max_hops: u32) -> crate::config::Config {
        parse(&format!(
            r#"
            [defaults]
            max_hops = {max_hops}

            [agents.a]
            runner = "claude"
            description = "a"
            prompt = "a"
            entry = true

            [agents.b]
            runner = "claude"
            description = "b"
            prompt = "b"

            [agents.c]
            runner = "claude"
            description = "c"
            prompt = "c"

            [agents.far]
            runner = "claude"
            description = "far"
            prompt = "far"

            [[routes]]
            from = "a"
            to = "b"

            [[routes]]
            from = "b"
            to = "c"

            [[routes]]
            from = "c"
            to = "far"
            "#
        ))
    }

    #[test]
    fn a_factory_with_no_entry_point_is_an_error() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            description = "plans"
            prompt = "plan"
            "#,
        );

        assert_mentions(&errors(&config), "nothing can be triggered");
    }

    #[test]
    fn a_pipeline_is_enough_to_make_a_factory_triggerable() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            description = "plans"
            prompt = "plan"
            access = "read-only"

            [pipelines.nightly]
            entry = "planner"
            trigger = { every = "1d" }
            "#,
        );

        assert_eq!(validate(&config), Vec::new());
    }

    #[test]
    fn an_unreachable_agent_warns() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            description = "plans"
            prompt = "plan"
            entry = true

            [agents.marooned]
            runner = "claude"
            description = "unreachable"
            prompt = "nobody calls me"
            "#,
        );

        assert_mentions(&warnings(&config), "`marooned` cannot be reached");
    }

    #[test]
    fn an_agent_deeper_than_the_hop_budget_warns() {
        let config = line_of_four(3);
        let warnings = warnings(&config);

        assert_mentions(&warnings, "`far` is 4 flights");
        assert_mentions(&warnings, "`max_hops` is 3");
        assert!(
            !warnings.iter().any(|m| m.contains("`c` is")),
            "`c` is woken by flight 3 and fits, so it must not be flagged"
        );
        assert!(
            !has_errors(&validate(&config)),
            "an under-sized hop budget is a warning, not a refusal to start"
        );
    }

    #[test]
    fn a_hop_budget_that_fits_is_not_flagged() {
        assert_eq!(validate(&line_of_four(4)), Vec::new());
    }

    #[test]
    fn a_loop_back_edge_does_not_shorten_the_measured_distance() {
        // The check measures shortest paths, so a rework loop is invisible to it. This pins the
        // limitation down rather than leaving it to be rediscovered: `max_hops = 2` is enough to
        // *reach* both agents once and nowhere near enough to turn the loop.
        let config = parse(
            r#"
            [defaults]
            max_hops = 2

            [agents.developer]
            runner = "claude"
            description = "builds"
            prompt = "build"
            entry = true

            [agents.reviewer]
            runner = "claude"
            description = "reviews"
            prompt = "review"
            access = "read-only"

            [[routes]]
            from = "developer"
            to = "reviewer"

            [[routes]]
            from = "reviewer"
            to = "developer"
            "#,
        );

        assert_eq!(
            validate(&config),
            Vec::new(),
            "shortest-path reach says nothing about how many times a loop can turn"
        );
    }

    #[test]
    fn a_barrier_its_upstreams_cannot_reach_warns() {
        // The case shortest-path reach alone misses. `target` looks three flights away through
        // `b`, but it is a `join = "all"`, so it waits for `c` too — and `c` wakes on flight 3
        // with nothing left to deliver with.
        let config = parse(
            r#"
            [defaults]
            max_hops = 3

            [agents.a]
            runner = "claude"
            description = "a"
            prompt = "a"
            entry = true

            [agents.b]
            runner = "claude"
            description = "b"
            prompt = "b"
            access = "read-only"

            [agents.c]
            runner = "claude"
            description = "c"
            prompt = "c"
            access = "read-only"

            [agents.target]
            runner = "claude"
            description = "target"
            prompt = "t"

            [[routes]]
            from = "a"
            to = "b"

            [[routes]]
            from = "b"
            to = "c"

            [[routes]]
            from = ["b", "c"]
            to = "target"
            join = "all"
            "#,
        );

        let warnings = warnings(&config);
        assert_mentions(&warnings, "`c` could only deliver on flight 4");
        assert_mentions(&warnings, "never release");
        assert!(
            !warnings.iter().any(|m| m.contains("`b` could only")),
            "`b` delivers on flight 3 and fits, so it must not be flagged: {warnings:?}"
        );
    }

    #[test]
    fn a_barrier_every_upstream_can_reach_is_not_flagged() {
        let config = parse(
            r#"
            [defaults]
            max_hops = 4

            [agents.a]
            runner = "claude"
            description = "a"
            prompt = "a"
            entry = true

            [agents.b]
            runner = "claude"
            description = "b"
            prompt = "b"
            access = "read-only"

            [agents.c]
            runner = "claude"
            description = "c"
            prompt = "c"
            access = "read-only"

            [agents.target]
            runner = "claude"
            description = "target"
            prompt = "t"

            [[routes]]
            from = "a"
            to = "b"

            [[routes]]
            from = "b"
            to = "c"

            [[routes]]
            from = ["b", "c"]
            to = "target"
            join = "all"
            "#,
        );

        assert_eq!(validate(&config), Vec::new());
    }
}
