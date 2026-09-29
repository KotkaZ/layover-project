//! Which routes a chain may use.
//!
//! A route may be scoped to pipelines, and a chain belongs to one pipeline or to none. This module
//! answers the question every route check asks — *may this chain send along this edge?* — with a
//! graph resolved for that chain.
//!
//! # Which pipeline a chain belongs to
//!
//! Always the Tower's answer, never an agent's: it is recorded from the trigger and carried on
//! queued work, and nothing an agent sends can change it.
//!
//! - A pipeline's trigger — dashboard, API or schedule — opens a chain belonging to it.
//! - A flight an agent sends continues its chain, and its pipeline.
//! - A `mode = "spawn"` flight opens a new chain that **inherits** the spawning chain's pipeline.
//!   A spawn gives a chain a fresh budget, not a fresh set of permissions.
//! - A resumed layover opens a chain belonging to the **resuming** pipeline, *narrowed* to what
//!   the chain that booked it could use — see [`ChainScope::resuming`].
//! - A flight sent straight to an `entry = true` agent belongs to no pipeline, and may use global
//!   routes only.
//!
//! # Why a resumed chain is narrowed
//!
//! Any agent may book a layover, and a resuming pipeline collects every layover that comes due.
//! Were the resumed chain given the resuming pipeline's routes outright, a chain from one workflow
//! could reach another's agents by setting its work down and waiting: an agent reading untrusted
//! text in a review sweep books a layover, and the follow-up pipeline wakes it with a route to the
//! agent that pushes code. So the resumed chain may use an edge only when every pipeline it
//! descends from permits it too. For the ordinary case — a follow-up pipeline resuming its own
//! workflow's work — both permit the same edges and nothing is lost.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::graph::RouteGraph;
use crate::pipeline::PipelineName;

/// Which routes one chain may use.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChainScope {
    /// The pipeline the chain belongs to. Its routes, joins and spawn edges are the chain's.
    ///
    /// `None` is a chain no pipeline opened, which may use global routes only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<PipelineName>,
    /// The pipelines of every chain this one descends from through a layover.
    ///
    /// An edge must be permitted in each of them as well as in [`ChainScope::pipeline`]. A `None`
    /// entry is an ancestor no pipeline opened, which narrows the chain to global routes. Empty
    /// for every chain that is not a resumed layover.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub within: BTreeSet<Option<PipelineName>>,
}

impl ChainScope {
    /// The scope of a chain belonging to `pipeline`, with no ancestry to narrow it.
    #[must_use]
    pub fn of(pipeline: Option<PipelineName>) -> Self {
        Self {
            pipeline,
            within: BTreeSet::new(),
        }
    }

    /// A scope with its ancestry, as recorded on queued work.
    #[must_use]
    pub fn new(pipeline: Option<PipelineName>, within: BTreeSet<Option<PipelineName>>) -> Self {
        let mut scope = Self { pipeline, within };
        scope.within.remove(&scope.pipeline.clone());
        scope
    }

    /// The scope of a chain the `resuming` pipeline opens for work a chain in `booked` set down.
    ///
    /// It belongs to the resuming pipeline — its flags, its joins, its labelling — and is narrowed
    /// to the booking chain's own scope and everything that was narrowing it. The ancestry is a
    /// set, so a layover set down and picked up a hundred times narrows no further than once.
    #[must_use]
    pub fn resuming(resuming: Option<PipelineName>, booked: &Self) -> Self {
        let mut within = booked.within.clone();
        within.insert(booked.pipeline.clone());
        Self::new(resuming, within)
    }
}

/// The route map, resolved once per pipeline.
///
/// Built when a factory is loaded and consulted on every send, every peer list, every dispatch
/// and every barrier, so that none of them builds a graph of its own — and none can reach for the
/// whole-factory graph by mistake.
#[derive(Debug, Clone, Default)]
pub struct RouteMap {
    everything: RouteGraph,
    global: RouteGraph,
    pipelines: BTreeMap<PipelineName, RouteGraph>,
}

impl RouteMap {
    /// Resolves a factory's routes for every pipeline it declares or names.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let mut names: BTreeSet<PipelineName> = config.pipelines.keys().cloned().collect();
        names.extend(
            config
                .routes
                .iter()
                .filter_map(|route| route.pipelines.as_ref())
                .flatten()
                .cloned(),
        );

        Self {
            everything: RouteGraph::from_config(config),
            global: RouteGraph::for_pipeline(config, None),
            pipelines: names
                .into_iter()
                .map(|name| {
                    let graph = RouteGraph::for_pipeline(config, Some(&name));
                    (name, graph)
                })
                .collect(),
        }
    }

    /// Every route, whatever its scope. For drawing the whole factory, never for a chain.
    #[must_use]
    pub fn everything(&self) -> &RouteGraph {
        &self.everything
    }

    /// The graph a chain belonging to `pipeline` uses, before any narrowing.
    ///
    /// A pipeline no route names and the factory does not declare — a chain queued before its
    /// pipeline was removed — gets the global routes, which is what the scoping rule gives it.
    #[must_use]
    pub fn for_pipeline(&self, pipeline: Option<&PipelineName>) -> &RouteGraph {
        pipeline
            .and_then(|name| self.pipelines.get(name))
            .unwrap_or(&self.global)
    }

    /// The graph one chain may use: its pipeline's, narrowed by its ancestry.
    #[must_use]
    pub fn for_scope(&self, scope: &ChainScope) -> Cow<'_, RouteGraph> {
        let own = self.for_pipeline(scope.pipeline.as_ref());
        if scope.within.is_empty() {
            return Cow::Borrowed(own);
        }

        let narrowed = scope.within.iter().fold(own.clone(), |graph, ancestor| {
            graph.restricted_to(self.for_pipeline(ancestor.as_ref()))
        });
        Cow::Owned(narrowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentName;

    fn routes(body: &str) -> RouteMap {
        let config = Config::from_toml(body, "scope-test.toml").expect("config parses");
        RouteMap::from_config(&config)
    }

    fn name(pipeline: &str) -> PipelineName {
        PipelineName::new(pipeline)
    }

    fn permits(graph: &RouteGraph, from: &str, to: &str) -> bool {
        graph.permits(&AgentName::new(from), &AgentName::new(to))
    }

    /// Two workflows sharing `eagle`: `devforge` may hand work to `bob`, `eagle-eye` may not.
    const SHARED: &str = r#"
        [pipelines.devforge]
        entry = "analyst"

        [pipelines.eagle-eye]
        entry = "azurix"

        [[routes]]
        from = "analyst"
        to = "eagle"

        [[routes]]
        from = "eagle"
        to = "bob"
        pipelines = "devforge"

        [[routes]]
        from = "azurix"
        to = "eagle"
        mode = "spawn"
        pipelines = ["eagle-eye"]

        [[routes]]
        from = "eagle"
        to = "azurix"
        pipelines = ["eagle-eye"]
    "#;

    #[test]
    fn a_global_route_applies_to_every_chain() {
        let map = routes(SHARED);

        for scope in [None, Some(name("devforge")), Some(name("eagle-eye"))] {
            assert!(
                permits(map.for_pipeline(scope.as_ref()), "analyst", "eagle"),
                "{scope:?}"
            );
        }
    }

    #[test]
    fn a_scoped_route_is_invisible_to_other_pipelines_and_to_a_chain_with_none() {
        let map = routes(SHARED);

        assert!(permits(
            map.for_pipeline(Some(&name("devforge"))),
            "eagle",
            "bob"
        ));
        assert!(!permits(
            map.for_pipeline(Some(&name("eagle-eye"))),
            "eagle",
            "bob"
        ));
        assert!(!permits(map.for_pipeline(None), "eagle", "bob"));
        assert!(
            permits(map.everything(), "eagle", "bob"),
            "the whole-factory map still draws it"
        );
    }

    #[test]
    fn a_spawn_edge_is_a_spawn_only_where_it_is_permitted() {
        let map = routes(SHARED);
        let eagle_eye = map.for_pipeline(Some(&name("eagle-eye")));

        assert!(eagle_eye.is_spawn(&"azurix".into(), &"eagle".into()));
        assert!(
            !map.for_pipeline(Some(&name("devforge")))
                .permits(&"azurix".into(), &"eagle".into())
        );
    }

    #[test]
    fn routes_for_the_same_pair_in_different_scopes_combine() {
        // The union applies: a pair two routes name for two pipelines is permitted in both.
        let map = routes(
            r#"
            [[routes]]
            from = "a"
            to = "b"
            pipelines = "one"

            [[routes]]
            from = "a"
            to = "b"
            pipelines = "two"
            "#,
        );

        assert!(permits(map.for_pipeline(Some(&name("one"))), "a", "b"));
        assert!(permits(map.for_pipeline(Some(&name("two"))), "a", "b"));
        assert!(!permits(map.for_pipeline(Some(&name("three"))), "a", "b"));
    }

    #[test]
    fn a_join_applies_only_in_its_scope() {
        let map = routes(
            r#"
            [[routes]]
            from = ["x", "y"]
            to = "c"
            join = "all"
            pipelines = "one"

            [[routes]]
            from = ["x", "y"]
            to = "c"
            pipelines = "two"
            "#,
        );

        assert!(
            map.for_pipeline(Some(&name("one")))
                .join_for(&"c".into())
                .is_some()
        );
        assert!(
            map.for_pipeline(Some(&name("two")))
                .join_for(&"c".into())
                .is_none()
        );
    }

    #[test]
    fn an_unknown_pipeline_gets_global_routes_only() {
        let map = routes(SHARED);

        let stale = map.for_pipeline(Some(&name("removed-since")));
        assert!(permits(stale, "analyst", "eagle"));
        assert!(!permits(stale, "eagle", "bob"));
    }

    #[test]
    fn a_resumed_chain_may_use_only_what_its_booking_chain_could() {
        // An Eagle Eye chain sets its work down; a follow-up pipeline that shares DevForge's
        // routes picks it up. Given DevForge's routes outright, `eagle` could hand `bob` work.
        let map = routes(
            r#"
            [pipelines.devforge]
            entry = "analyst"

            [pipelines.follow-up]
            entry = "azurix"
            resumes = true

            [pipelines.eagle-eye]
            entry = "azurix"

            [[routes]]
            from = "eagle"
            to = "bob"
            pipelines = ["devforge", "follow-up"]

            [[routes]]
            from = "eagle"
            to = "sherlock"
            "#,
        );

        let from_eagle_eye = ChainScope::resuming(
            Some(name("follow-up")),
            &ChainScope::of(Some(name("eagle-eye"))),
        );
        let from_devforge = ChainScope::resuming(
            Some(name("follow-up")),
            &ChainScope::of(Some(name("devforge"))),
        );

        assert!(!permits(&map.for_scope(&from_eagle_eye), "eagle", "bob"));
        assert!(permits(
            &map.for_scope(&from_eagle_eye),
            "eagle",
            "sherlock"
        ));
        assert!(
            permits(&map.for_scope(&from_devforge), "eagle", "bob"),
            "the ordinary follow-up keeps everything its workflow had"
        );
        assert_eq!(from_eagle_eye.pipeline, Some(name("follow-up")));
    }

    #[test]
    fn narrowing_accumulates_across_layovers_and_does_not_grow() {
        let booked = ChainScope::of(Some(name("eagle-eye")));
        let once = ChainScope::resuming(Some(name("follow-up")), &booked);
        let twice = ChainScope::resuming(Some(name("follow-up")), &once);
        let again = ChainScope::resuming(Some(name("follow-up")), &twice);

        assert!(twice.within.contains(&Some(name("eagle-eye"))), "{twice:?}");
        assert_eq!(again, twice, "setting work down again narrows no further");
    }

    #[test]
    fn a_layover_booked_by_a_chain_with_no_pipeline_narrows_to_global_routes() {
        let map = routes(SHARED);
        let resumed = ChainScope::resuming(Some(name("devforge")), &ChainScope::of(None));

        assert!(!permits(&map.for_scope(&resumed), "eagle", "bob"));
        assert!(permits(&map.for_scope(&resumed), "analyst", "eagle"));
    }

    #[test]
    fn a_scope_round_trips_through_storage_and_says_nothing_when_empty() {
        let scope = ChainScope::resuming(Some(name("follow-up")), &ChainScope::of(None));
        let json = serde_json::to_string(&scope).expect("serialises");
        let read: ChainScope = serde_json::from_str(&json).expect("deserialises");

        assert_eq!(read, scope);
        assert_eq!(
            serde_json::to_string(&ChainScope::default()).expect("serialises"),
            "{}"
        );
    }
}
