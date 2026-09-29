//! The route map as a directed graph.
//!
//! The graph answers two questions the Tower asks constantly: whether an edge is permitted, and
//! which agents could still be reached from a given set of live runs. The second is what allows
//! an unreachable rendezvous barrier to be abandoned rather than parked forever.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::agent::AgentName;
use crate::config::Config;
use crate::pipeline::PipelineName;
use crate::route::{Join, Route};

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
    /// Edges that open a new itinerary rather than continuing the current one.
    ///
    /// Kept apart from `edges` because a spawn is a permission like any other but a *distance* of
    /// a different kind: the receiver starts a fresh chain with fresh Hops, so counting a spawn as
    /// one more step of the same chain measures something that does not exist.
    spawns: BTreeMap<AgentName, BTreeSet<AgentName>>,
    joins: BTreeMap<AgentName, JoinSpec>,
}

impl RouteGraph {
    /// Builds a graph of **every** route, whatever pipelines it is scoped to.
    ///
    /// This is the factory drawn as one map, for questions about the factory as a whole — the
    /// whole-factory diagram, what any chain could possibly do. It is not what any one chain may
    /// use: a chain uses the graph of its own pipeline, from [`RouteGraph::for_pipeline`] or
    /// [`crate::scope::RouteMap`]. For a factory that scopes nothing the two are the same graph.
    ///
    /// Routes that name several senders and several receivers expand to the full cross product,
    /// which is what makes `from = ["a", "b"]` with `to = "c"` a rendezvous and `from = "a"` with
    /// `to = ["b", "c"]` a fan-out.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        Self::from_routes(config.routes.iter())
    }

    /// Builds the graph a chain belonging to `pipeline` may use: every global route, and every
    /// route scoped to that pipeline.
    ///
    /// `None` is a chain no pipeline opened, which gets global routes only.
    #[must_use]
    pub fn for_pipeline(config: &Config, pipeline: Option<&PipelineName>) -> Self {
        Self::from_routes(
            config
                .routes
                .iter()
                .filter(|route| route.applies_to(pipeline)),
        )
    }

    /// Builds a graph from exactly these routes.
    fn from_routes<'a>(routes: impl IntoIterator<Item = &'a Route>) -> Self {
        let mut graph = Self::default();

        for route in routes {
            for from in &route.from {
                for to in &route.to {
                    graph
                        .edges
                        .entry(from.clone())
                        .or_default()
                        .insert(to.clone());

                    if route.is_spawn() {
                        graph
                            .spawns
                            .entry(from.clone())
                            .or_default()
                            .insert(to.clone());
                    }
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

    /// Keeps only the edges `other` also permits.
    ///
    /// For a chain resuming work another pipeline set down: it may use a route only when both the
    /// pipeline resuming it and every pipeline it descends from permit it — see
    /// [`crate::scope::ChainScope`]. Spawn edges and joins stay this graph's own, because they
    /// describe how *this* chain's pipeline delivers, and `other` only narrows who it may reach.
    #[must_use]
    pub fn restricted_to(&self, other: &Self) -> Self {
        let keep = |edges: &BTreeMap<AgentName, BTreeSet<AgentName>>| {
            edges
                .iter()
                .map(|(from, tos)| {
                    let kept: BTreeSet<AgentName> = tos
                        .iter()
                        .filter(|to| other.permits(from, to))
                        .cloned()
                        .collect();
                    (from.clone(), kept)
                })
                .filter(|(_, tos)| !tos.is_empty())
                .collect()
        };

        Self {
            edges: keep(&self.edges),
            spawns: keep(&self.spawns),
            joins: self.joins.clone(),
        }
    }

    /// Returns `true` when this edge opens a new itinerary rather than continuing one.
    #[must_use]
    pub fn is_spawn(&self, from: &AgentName, to: &AgentName) -> bool {
        self.spawns.get(from).is_some_and(|tos| tos.contains(to))
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

    /// Every agent reached by a spawn edge.
    ///
    /// These begin chains of their own, so for any question about hop depth they are entry points
    /// rather than destinations.
    pub fn spawn_targets(&self) -> impl Iterator<Item = &AgentName> {
        self.spawns.values().flatten()
    }

    /// Every agent belonging to the workflow that starts at `entry`.
    ///
    /// Unlike [`RouteGraph::reachable_from`] this **does** cross spawn edges. The two questions
    /// are different: reachability asks what could still deliver into *this* itinerary, and a
    /// spawned chain never can. Workflow membership asks what this way in sets in motion, and a
    /// reviewer spawned by a sweep is unarguably part of the sweep.
    ///
    /// Agents shared between workflows appear in both, which is the honest answer — the
    /// developer really is in the triage pipeline and the follow-up pipeline.
    #[must_use]
    pub fn workflow_from(&self, entry: &AgentName) -> BTreeSet<AgentName> {
        self.workflow_from_all([entry])
    }

    /// [`RouteGraph::workflow_from`], from several starting agents at once.
    #[must_use]
    pub fn workflow_from_all<'a>(
        &self,
        sources: impl IntoIterator<Item = &'a AgentName>,
    ) -> BTreeSet<AgentName> {
        let mut seen: BTreeSet<AgentName> = sources.into_iter().cloned().collect();
        let mut queue: VecDeque<AgentName> = seen.iter().cloned().collect();

        while let Some(current) = queue.pop_front() {
            for next in self.successors(&current) {
                if seen.insert(next.clone()) {
                    queue.push_back(next.clone());
                }
            }
        }

        seen
    }

    /// Every spawn edge, as a sender/receiver pair.
    pub fn spawn_edges(&self) -> impl Iterator<Item = (&AgentName, &AgentName)> {
        self.spawns
            .iter()
            .flat_map(|(from, tos)| tos.iter().map(move |to| (from, to)))
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
        self.distances_from(sources).into_keys().collect()
    }

    /// Returns every agent reachable from `sources`, each with its distance in edges.
    ///
    /// The sources sit at distance zero. An agent at distance `d` is therefore woken by flight
    /// `d + 1` of a chain that began at a source, which is what makes the figure directly
    /// comparable against `max_hops` — see [`mod@crate::validate`].
    ///
    /// Shortest paths are an optimistic bound. A factory whose agents loop will spend far more
    /// hops than the distance suggests, so this proves an agent *can* be reached, never that a
    /// particular itinerary will get there.
    pub fn distances_from<'a>(
        &self,
        sources: impl IntoIterator<Item = &'a AgentName>,
    ) -> BTreeMap<AgentName, u32> {
        let mut seen: BTreeMap<AgentName, u32> = BTreeMap::new();
        let mut queue: VecDeque<AgentName> = VecDeque::new();

        for source in sources {
            if !seen.contains_key(source) {
                seen.insert(source.clone(), 0);
                queue.push_back(source.clone());
            }
        }

        while let Some(current) = queue.pop_front() {
            let depth = seen[&current];
            for next in self.successors(&current) {
                // A spawn edge is a permission, not a step. The receiver starts a fresh itinerary
                // with fresh Hops, so counting it as one more hop of this chain measures a
                // distance that does not exist — and for barrier abandonment it is worse than
                // wrong: a spawned agent runs under a different itinerary and can never deliver
                // to this one's barrier, so treating it as reachable keeps a dead barrier parked.
                if self.is_spawn(&current, next) {
                    continue;
                }
                if !seen.contains_key(next) {
                    seen.insert(next.clone(), depth + 1);
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

    #[test]
    fn distance_counts_edges_from_the_source() {
        let graph = graph_from(
            r#"
            [[routes]]
            from = "planner"
            to = "probe_a"

            [[routes]]
            from = "probe_a"
            to = "collector"
            "#,
        );

        let distances = graph.distances_from([&AgentName::from("planner")]);

        assert_eq!(distances[&AgentName::from("planner")], 0);
        assert_eq!(distances[&AgentName::from("probe_a")], 1);
        assert_eq!(distances[&AgentName::from("collector")], 2);
    }

    #[test]
    fn distance_is_the_shortest_path_not_the_longest() {
        // `collector` is two edges away the long way round and one edge away directly. Hops are
        // spent along whichever path an agent actually chooses, so the short answer is a bound
        // and not a prediction.
        let graph = graph_from(
            r#"
            [[routes]]
            from = "planner"
            to = ["probe_a", "collector"]

            [[routes]]
            from = "probe_a"
            to = "collector"
            "#,
        );

        let distances = graph.distances_from([&AgentName::from("planner")]);

        assert_eq!(distances[&AgentName::from("collector")], 1);
    }

    #[test]
    fn distance_omits_agents_no_edge_leads_to() {
        let graph = graph_from(
            r#"
            [[routes]]
            from = "planner"
            to = "probe_a"

            [[routes]]
            from = "orphan"
            to = "elsewhere"
            "#,
        );

        let distances = graph.distances_from([&AgentName::from("planner")]);

        assert!(!distances.contains_key(&AgentName::from("orphan")));
    }
}
