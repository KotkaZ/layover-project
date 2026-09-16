//! Checks about the route map: unknown agents, ambiguous joins, and unsafe fan-out.

use std::collections::BTreeMap;

use crate::agent::{Access, AgentName};
use crate::config::Config;

use super::Diagnostic;

pub(super) fn check_routes_name_known_agents(config: &Config, found: &mut Vec<Diagnostic>) {
    for (index, route) in config.routes.iter().enumerate() {
        if route.from.is_empty() || route.to.is_empty() {
            found.push(Diagnostic::error(format!(
                "route {index} has an empty `from` or `to`"
            )));
        }

        for name in route.from.iter().chain(route.to.iter()) {
            if !config.agents.contains_key(name) {
                found.push(Diagnostic::error(format!(
                    "route {index} names unknown agent `{name}`"
                )));
            }
        }
    }
}

pub(super) fn check_joins_are_unambiguous(config: &Config, found: &mut Vec<Diagnostic>) {
    let mut join_targets: BTreeMap<&AgentName, usize> = BTreeMap::new();

    for (index, route) in config.routes.iter().enumerate() {
        if !route.is_join() {
            continue;
        }

        if route.to.len() > 1 {
            found.push(Diagnostic::error(format!(
                "route {index} declares a join with several targets; a barrier guards exactly one agent"
            )));
        }

        if route.from.len() < 2 {
            found.push(Diagnostic::warning(format!(
                "route {index} declares a join with fewer than two upstreams, which has no effect"
            )));
        }

        for target in &route.to {
            *join_targets.entry(target).or_default() += 1;
        }
    }

    for (target, count) in join_targets {
        if count > 1 {
            found.push(Diagnostic::error(format!(
                "agent `{target}` is the target of {count} join routes; which barrier applies is undefined"
            )));
        }
    }
}

/// A spawn edge cannot also be a rendezvous.
///
/// A barrier holds flights until every named upstream has arrived *in the same itinerary*, and a
/// spawn opens a new one per flight — so each spawned chain would arrive at a barrier alone and
/// park there forever. It is not a judgement call or a smell: the two mean opposite things, and
/// the result is silently stalled work rather than an error anybody would see.
pub(super) fn check_spawns_do_not_join(config: &Config, found: &mut Vec<Diagnostic>) {
    for (index, route) in config.routes.iter().enumerate() {
        if route.is_spawn() && route.is_join() {
            found.push(Diagnostic::error(format!(
                "route {index} both spawns and joins. A spawn opens a new itinerary per flight, \
                 so each one would reach the barrier alone and park there forever. Use `mode = \
                 \"spawn\"` or `join`, not both"
            )));
        }
    }
}

pub(super) fn check_read_write_fan_out(config: &Config, found: &mut Vec<Diagnostic>) {
    for (index, route) in config.routes.iter().enumerate() {
        if route.to.len() < 2 {
            continue;
        }

        let writers: Vec<&AgentName> = route
            .to
            .iter()
            .filter(|name| {
                config
                    .agents
                    .get(*name)
                    .is_some_and(|agent| agent.access == Access::ReadWrite)
            })
            .collect();

        if writers.len() > 1 {
            let names = writers
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ");
            found.push(Diagnostic::warning(format!(
                "route {index} fans out to several read-write agents ({names}); they share one \
                 working directory and will overwrite each other"
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::validate::testing::{assert_mentions, errors, parse, warnings};
    use crate::validate::{has_errors, validate};

    const AGENTS: &str = r#"
        [agents.planner]
        runner = "claude"
        description = "plans"
        prompt = "plan"
        entry = true

        [agents.probe_a]
        runner = "claude"
        description = "probes"
        prompt = "a"

        [agents.probe_b]
        runner = "claude"
        description = "probes"
        prompt = "b"

        [agents.collector]
        runner = "claude"
        description = "collects"
        prompt = "collect"
    "#;

    #[test]
    fn an_unknown_agent_in_a_route_is_an_error() {
        let config = parse(&format!(
            r#"
            {AGENTS}

            [[routes]]
            from = "planner"
            to = "ghost"
            "#
        ));

        assert!(has_errors(&validate(&config)));
        assert_mentions(&errors(&config), "unknown agent `ghost`");
    }

    #[test]
    fn two_join_routes_targeting_one_agent_are_rejected() {
        let config = parse(&format!(
            r#"
            {AGENTS}

            [[routes]]
            from = "planner"
            to = ["probe_a", "probe_b"]

            [[routes]]
            from = ["probe_a", "probe_b"]
            to = "collector"
            join = "all"

            [[routes]]
            from = ["planner", "probe_a"]
            to = "collector"
            join = "any"
            "#
        ));

        assert_mentions(&errors(&config), "which barrier applies is undefined");
    }

    #[test]
    fn a_join_with_several_targets_is_rejected() {
        let config = parse(&format!(
            r#"
            {AGENTS}

            [[routes]]
            from = ["probe_a", "probe_b"]
            to = ["collector", "planner"]
            join = "all"
            "#
        ));

        assert_mentions(&errors(&config), "a barrier guards exactly one agent");
    }

    #[test]
    fn a_join_with_one_upstream_warns() {
        let config = parse(&format!(
            r#"
            {AGENTS}

            [[routes]]
            from = "planner"
            to = "collector"
            join = "all"
            "#
        ));

        assert_mentions(&warnings(&config), "fewer than two upstreams");
    }

    #[test]
    fn a_route_that_both_spawns_and_joins_is_rejected() {
        // Not a judgement call. A barrier waits for upstreams within one itinerary and a spawn
        // opens a new one per flight, so every spawned chain would reach the barrier alone and
        // park there forever -- silently stalled work rather than an error anybody would see.
        let config = parse(
            r#"
            [agents.scanner]
            prompt = "scan"
            entry = true

            [agents.left]
            prompt = "left"

            [agents.right]
            prompt = "right"

            [[routes]]
            from = ["left", "right"]
            to = "scanner"
            join = "all"
            mode = "spawn"
            "#,
        );

        assert_mentions(&errors(&config), "both spawns and joins");
    }

    #[test]
    fn spawning_without_a_join_is_fine() {
        let config = parse(
            r#"
            [agents.scanner]
            prompt = "scan"
            entry = true

            [agents.reviewer]
            prompt = "review"

            [[routes]]
            from = "scanner"
            to = "reviewer"
            mode = "spawn"
            "#,
        );

        assert!(
            !errors(&config)
                .iter()
                .any(|message| message.contains("spawns and joins")),
            "a plain spawn edge is the normal case: {:?}",
            errors(&config)
        );
    }

    #[test]
    fn fanning_out_to_two_writers_warns() {
        let config = parse(&format!(
            r#"
            {AGENTS}

            [[routes]]
            from = "planner"
            to = ["probe_a", "probe_b"]
            "#
        ));

        assert!(!has_errors(&validate(&config)));
        assert_mentions(&warnings(&config), "read-write");
    }

    #[test]
    fn a_read_only_sibling_makes_the_fan_out_safe() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            description = "plans"
            prompt = "plan"
            entry = true

            [agents.probe_a]
            runner = "claude"
            description = "writes"
            prompt = "a"

            [agents.probe_b]
            runner = "claude"
            description = "reads"
            prompt = "b"
            access = "read-only"

            [[routes]]
            from = "planner"
            to = ["probe_a", "probe_b"]
            "#,
        );

        assert_eq!(validate(&config), Vec::new());
    }
}
