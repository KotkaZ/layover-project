//! Load-time validation of a factory definition.
//!
//! Everything here runs before the first flight, because an unattended factory that discovers a
//! typo three agents deep has already spent money to find out. Errors block startup; warnings
//! describe shapes that are legal but known to be hazardous.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::{Access, AgentName, Config};
use crate::graph::RouteGraph;

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The factory must not start.
    Error,
    /// Legal, but likely to misbehave.
    Warning,
}

/// A single validation finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// How serious it is.
    pub severity: Severity,
    /// Human-readable explanation.
    pub message: String,
}

impl Diagnostic {
    /// Builds an error.
    fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
        }
    }

    /// Builds a warning.
    fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
        }
    }

    /// Returns `true` if this finding should block startup.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// Returns `true` if any finding blocks startup.
#[must_use]
pub fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(Diagnostic::is_error)
}

/// Checks a factory definition for problems.
#[must_use]
pub fn validate(config: &Config) -> Vec<Diagnostic> {
    let mut found = Vec::new();

    check_routes_name_known_agents(config, &mut found);
    check_runners_exist(config, &mut found);
    check_entry_points(config, &mut found);
    check_joins_are_unambiguous(config, &mut found);
    check_read_write_fan_out(config, &mut found);
    check_reachability(config, &mut found);

    found
}

fn check_routes_name_known_agents(config: &Config, found: &mut Vec<Diagnostic>) {
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

fn check_runners_exist(config: &Config, found: &mut Vec<Diagnostic>) {
    for (name, agent) in &config.agents {
        let runner = agent.runner.as_ref().or(config.defaults.runner.as_ref());

        match runner {
            None => found.push(Diagnostic::error(format!(
                "agent `{name}` has no runner and no default runner is set"
            ))),
            Some(runner) if !config.runners.contains_key(runner) => {
                found.push(Diagnostic::error(format!(
                    "agent `{name}` uses unknown runner `{runner}`"
                )));
            }
            Some(_) => {}
        }
    }
}

fn check_entry_points(config: &Config, found: &mut Vec<Diagnostic>) {
    if config.agents.is_empty() {
        found.push(Diagnostic::error("the factory defines no agents"));
        return;
    }

    if config.entry_agents().next().is_none() {
        found.push(Diagnostic::error(
            "no agent is marked `entry = true`, so nothing can be triggered",
        ));
    }
}

fn check_joins_are_unambiguous(config: &Config, found: &mut Vec<Diagnostic>) {
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

fn check_read_write_fan_out(config: &Config, found: &mut Vec<Diagnostic>) {
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

fn check_reachability(config: &Config, found: &mut Vec<Diagnostic>) {
    let entries: Vec<&AgentName> = config.entry_agents().collect();
    if entries.is_empty() {
        return;
    }

    let graph = RouteGraph::from_config(config);
    let reachable: BTreeSet<AgentName> = graph.reachable_from(entries);

    for name in config.agents.keys() {
        if !reachable.contains(name) {
            found.push(Diagnostic::warning(format!(
                "agent `{name}` cannot be reached from any entry point"
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNER: &str = r#"
        [runners.claude]
        command = ["claude", "-p", "{prompt}"]
    "#;

    fn parse(body: &str) -> Config {
        Config::from_toml(&format!("{RUNNER}{body}"), "test.toml").expect("config parses")
    }

    fn messages(config: &Config, severity: Severity) -> Vec<String> {
        validate(config)
            .into_iter()
            .filter(|d| d.severity == severity)
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn a_minimal_factory_is_clean() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            prompt = "plan"
            entry = true

            [agents.coder]
            runner = "claude"
            prompt = "code"

            [[routes]]
            from = "planner"
            to = "coder"
            "#,
        );

        assert_eq!(validate(&config), Vec::new());
    }

    #[test]
    fn an_unknown_agent_in_a_route_is_an_error() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            prompt = "plan"
            entry = true

            [[routes]]
            from = "planner"
            to = "ghost"
            "#,
        );

        assert!(has_errors(&validate(&config)));
        assert!(
            messages(&config, Severity::Error)
                .iter()
                .any(|m| m.contains("unknown agent `ghost`"))
        );
    }

    #[test]
    fn an_unknown_runner_is_an_error() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "nonesuch"
            prompt = "plan"
            entry = true
            "#,
        );

        assert!(
            messages(&config, Severity::Error)
                .iter()
                .any(|m| m.contains("unknown runner `nonesuch`"))
        );
    }

    #[test]
    fn a_factory_with_no_entry_point_is_an_error() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            prompt = "plan"
            "#,
        );

        assert!(
            messages(&config, Severity::Error)
                .iter()
                .any(|m| m.contains("entry = true"))
        );
    }

    #[test]
    fn two_join_routes_targeting_one_agent_are_rejected() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            prompt = "plan"
            entry = true

            [agents.probe_a]
            runner = "claude"
            prompt = "a"

            [agents.probe_b]
            runner = "claude"
            prompt = "b"

            [agents.collector]
            runner = "claude"
            prompt = "collect"

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
            "#,
        );

        assert!(
            messages(&config, Severity::Error)
                .iter()
                .any(|m| m.contains("which barrier applies is undefined"))
        );
    }

    #[test]
    fn fanning_out_to_two_writers_warns() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            prompt = "plan"
            entry = true

            [agents.probe_a]
            runner = "claude"
            prompt = "a"

            [agents.probe_b]
            runner = "claude"
            prompt = "b"

            [[routes]]
            from = "planner"
            to = ["probe_a", "probe_b"]
            "#,
        );

        assert!(!has_errors(&validate(&config)));
        assert!(
            messages(&config, Severity::Warning)
                .iter()
                .any(|m| m.contains("read-write"))
        );
    }

    #[test]
    fn a_read_only_sibling_makes_the_fan_out_safe() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            prompt = "plan"
            entry = true

            [agents.probe_a]
            runner = "claude"
            prompt = "a"

            [agents.probe_b]
            runner = "claude"
            prompt = "b"
            access = "read-only"

            [[routes]]
            from = "planner"
            to = ["probe_a", "probe_b"]
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
            prompt = "plan"
            entry = true

            [agents.marooned]
            runner = "claude"
            prompt = "nobody calls me"
            "#,
        );

        assert!(
            messages(&config, Severity::Warning)
                .iter()
                .any(|m| m.contains("`marooned` cannot be reached"))
        );
    }
}
