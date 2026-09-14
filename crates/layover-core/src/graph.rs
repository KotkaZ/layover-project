//! The route map as a directed graph.
//!
//! The graph answers two questions the Tower asks constantly: whether an edge is permitted, and
//! which agents could still be reached from a given set of live runs. The second is what allows
//! an unreachable rendezvous barrier to be abandoned rather than parked forever.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::config::{AgentName, Config, Join};

/// The rendezvous condition attached to a receiving agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinSpec {
    /// Agents whose flights are parked until the condition is met.
    pub upstreams: BTreeSet<AgentName>,
    /// The release condition.
    pub join: Join,
    /// Backstop for a barrier that never becomes unreachable but also never completes.
    pub timeout_sec: Option<u64>,
}

/// The route map, indexed for traversal.
#[derive(Debug, Clone, Default)]
pub struct RouteGraph {
    edges: BTreeMap<AgentName, BTreeSet<AgentName>>,
    joins: BTreeMap<AgentName, JoinSpec>,
}

impl RouteGraph {
    /// Builds a graph from a factory definition.
    ///
    /// Routes that name several senders and several receivers expand to the full cross product,
    /// which is what makes `from = ["a", "b"]` with `to = "c"` a rendezvous and `from = "a"` with
    /// `to = ["b", "c"]` a fan-out.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let mut graph = Self::default();

        for route in &config.routes {
            for from in &route.from {
                for to in &route.to {
                    graph
                        .edges
                        .entry(from.clone())
                        .or_default()
                        .insert(to.clone());
                }
            }

            if let Some(join) = route.join {
                for to in &route.to {
                    graph.joins.insert(
                        to.clone(),
                        JoinSpec {
                            upstreams: route.from.iter().cloned().collect(),
                            join,
                            timeout_sec: route.timeout_sec,
                        },
                    );
                }
            }
        }

        graph
    }

    /// Returns `true` if `from` is permitted to send to `to`.
    #[must_use]
    pub fn permits(&self, from: &AgentName, to: &AgentName) -> bool {
        self.edges.get(from).is_some_and(|tos| tos.contains(to))
    }

    /// Returns the agents `from` may send to.
    pub fn successors(&self, from: &AgentName) -> impl Iterator<Item = &AgentName> {
        self.edges.get(from).into_iter().flatten()
    }

    /// Returns the rendezvous condition guarding `agent`, if any.
    #[must_use]
    pub fn join_for(&self, agent: &AgentName) -> Option<&JoinSpec> {
        self.joins.get(agent)
    }

    /// Returns every agent reachable from `sources`, including the sources themselves.
    ///
    /// Hops are deliberately ignored, which makes the answer conservative: an agent that Hops
    /// would actually prevent from being reached is still reported as reachable. A barrier is
    /// therefore never abandoned prematurely, and `timeout_sec` remains the backstop.
    pub fn reachable_from<'a>(
        &self,
        sources: impl IntoIterator<Item = &'a AgentName>,
    ) -> BTreeSet<AgentName> {
        let mut seen = BTreeSet::new();
        let mut queue: VecDeque<AgentName> = VecDeque::new();

        for source in sources {
            if seen.insert(source.clone()) {
                queue.push_back(source.clone());
            }
        }

        while let Some(current) = queue.pop_front() {
            for next in self.successors(&current) {
                if seen.insert(next.clone()) {
                    queue.push_back(next.clone());
                }
            }
        }

        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_from(toml: &str) -> RouteGraph {
        let config = Config::from_toml(toml, "test.toml").expect("config parses");
        RouteGraph::from_config(&config)
    }

    #[test]
    fn edges_are_directed() {
        let graph = graph_from(
            r#"
            [[routes]]
            from = "planner"
            to = "coder"
            "#,
        );

        assert!(graph.permits(&"planner".into(), &"coder".into()));
        assert!(!graph.permits(&"coder".into(), &"planner".into()));
    }

    #[test]
    fn fan_out_expands_to_one_edge_per_target() {
        let graph = graph_from(
            r#"
            [[routes]]
            from = "planner"
            to = ["probe_a", "probe_b"]
            "#,
        );

        assert!(graph.permits(&"planner".into(), &"probe_a".into()));
        assert!(graph.permits(&"planner".into(), &"probe_b".into()));
        assert!(graph.join_for(&"probe_a".into()).is_none());
    }

    #[test]
    fn join_route_records_every_upstream() {
        let graph = graph_from(
            r#"
            [[routes]]
            from = ["probe_a", "probe_b"]
            to = "collector"
            join = "all"
            timeout_sec = 60
            "#,
        );

        let spec = graph.join_for(&"collector".into()).expect("join recorded");
        assert_eq!(spec.join, Join::All);
        assert_eq!(spec.timeout_sec, Some(60));
        assert!(spec.upstreams.contains(&"probe_a".into()));
        assert!(spec.upstreams.contains(&"probe_b".into()));
    }

    #[test]
    fn reachability_follows_edges_transitively() {
        let graph = graph_from(
            r#"
            [[routes]]
            from = "planner"
            to = "probe_a"

            [[routes]]
            from = "probe_a"
            to = "collector"

            [[routes]]
            from = "orphan"
            to = "elsewhere"
            "#,
        );

        let reachable = graph.reachable_from([&AgentName::from("planner")]);

        assert!(reachable.contains(&"planner".into()));
        assert!(reachable.contains(&"collector".into()));
        assert!(!reachable.contains(&"orphan".into()));
    }

    #[test]
    fn reachability_terminates_on_cycles() {
        let graph = graph_from(
            r#"
            [[routes]]
            from = "a"
            to = "b"

            [[routes]]
            from = "b"
            to = "a"
            "#,
        );

        let reachable = graph.reachable_from([&AgentName::from("a")]);
        assert_eq!(reachable.len(), 2);
    }
}
