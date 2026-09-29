//! Edges in the route map.
//!
//! An edge is a *permission*, not a step in a pipeline: it says agent A may send to agent B, and
//! nothing about when or whether it will. Rendezvous joins are the one qualification, and they
//! constrain the receiving node rather than the sender — see [`crate::barrier`].
//!
//! A route may be scoped to pipelines. It is then a permission *within those workflows*: a chain
//! started by another pipeline cannot use it, however its agents are persuaded.

use serde::{Deserialize, Serialize};

use crate::agent::AgentName;
use crate::pipeline::PipelineName;

/// The condition under which a rendezvous barrier releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Join {
    /// Wait for every declared upstream.
    All,
    /// Release as soon as any one upstream arrives.
    Any,
}

/// Delivery semantics for an edge.
///
/// v0.1 has a single mode. Blocking request/response was superseded by rendezvous joins, which
/// park flights instead of parking processes. The field exists so that a configuration written
/// against the older design fails with a clear message rather than an unknown-field error.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Fire-and-forget within the current itinerary. The sender continues immediately.
    #[default]
    Async,
    /// Opens a *new* itinerary rather than continuing this one.
    ///
    /// The receiver gets its own Fuel, its own hop budget and its own workspace. That is what
    /// makes per-item work affordable: a scanner dispatching one reviewer per pull request over
    /// an `async` edge would put every reviewer on one budget, so the sweep would stop partway
    /// and which pull requests got reviewed would be arbitrary.
    ///
    /// It is a route rather than a free-standing capability because the route map is the single
    /// source of truth for who may reach whom. A spawn that skipped it would be an unchecked
    /// edge — and an agent able to open a fresh, fully funded chain into any peer is a larger
    /// hole than one able to send it a message.
    Spawn,
}

impl Mode {
    /// Returns `true` when this edge opens a new itinerary.
    #[must_use]
    pub fn is_spawn(self) -> bool {
        matches!(self, Self::Spawn)
    }
}

/// A directed edge, or set of edges, in the route map.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    /// Sending agents. Accepts a bare string or a list.
    #[serde(deserialize_with = "one_or_many")]
    pub from: Vec<AgentName>,
    /// Receiving agents. Accepts a bare string or a list.
    #[serde(deserialize_with = "one_or_many")]
    pub to: Vec<AgentName>,
    /// Delivery semantics.
    #[serde(default)]
    pub mode: Mode,
    /// Rendezvous condition. When set, flights are parked until it is met.
    #[serde(default)]
    pub join: Option<Join>,
    /// Backstop for an unreachable barrier.
    #[serde(default)]
    pub timeout_sec: Option<u64>,
    /// The pipelines whose chains may use this route. Accepts a bare string or a list.
    ///
    /// Absent means **global**: every chain may use it, which is what every route meant before
    /// scopes existed. Present, only chains belonging to one of the named pipelines may — so a
    /// route stays a permission, now within a workflow rather than across the whole factory. It
    /// says nothing about order. See [`crate::scope`] for which pipeline a chain belongs to.
    ///
    /// `Some` of an empty list is kept distinct from absent so that validation can refuse it:
    /// read as "no pipelines" it is a route nothing may use, and read as "global" it is the widest
    /// permission there is. Neither reading is safe to guess.
    #[serde(default, deserialize_with = "some_one_or_many")]
    pub pipelines: Option<Vec<PipelineName>>,
}

impl Route {
    /// Returns `true` when this edge opens a new itinerary per flight.
    #[must_use]
    pub fn is_spawn(&self) -> bool {
        self.mode.is_spawn()
    }

    /// Returns `true` when this edge parks flights at a barrier.
    #[must_use]
    pub fn is_join(&self) -> bool {
        self.join.is_some()
    }

    /// Returns `true` when this route names the pipelines it belongs to.
    #[must_use]
    pub fn is_scoped(&self) -> bool {
        self.pipelines.is_some()
    }

    /// Whether a chain belonging to `pipeline` may use this route.
    ///
    /// A chain no pipeline opened — a flight sent straight to an `entry = true` agent — passes
    /// `None`, and may use global routes only.
    #[must_use]
    pub fn applies_to(&self, pipeline: Option<&PipelineName>) -> bool {
        match &self.pipelines {
            None => true,
            Some(names) => pipeline.is_some_and(|wanted| names.contains(wanted)),
        }
    }

    /// Whether some chain could use both this route and `other`.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        match (&self.pipelines, &other.pipelines) {
            (None, _) | (_, None) => true,
            (Some(mine), Some(theirs)) => mine.iter().any(|name| theirs.contains(name)),
        }
    }

    /// The pipelines both routes apply to, for saying where two routes collide.
    ///
    /// Empty when both are global, which is every chain rather than none.
    #[must_use]
    pub fn shared_pipelines(&self, other: &Self) -> Vec<PipelineName> {
        match (&self.pipelines, &other.pipelines) {
            (None, None) => Vec::new(),
            (Some(names), None) | (None, Some(names)) => names.clone(),
            (Some(mine), Some(theirs)) => mine
                .iter()
                .filter(|name| theirs.contains(name))
                .cloned()
                .collect(),
        }
    }
}

/// Accepts either `from = "a"` or `from = ["a", "b"]`.
fn one_or_many<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw<T> {
        One(T),
        Many(Vec<T>),
    }

    Ok(match Raw::deserialize(deserializer)? {
        Raw::One(name) => vec![name],
        Raw::Many(names) => names,
    })
}

/// [`one_or_many`] for a key that may be left out altogether.
fn some_one_or_many<'de, D, T>(deserializer: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    one_or_many(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(body: &str) -> Route {
        toml::from_str(body).expect("route parses")
    }

    #[test]
    fn from_and_to_accept_a_string_or_a_list() {
        let route = route(
            r#"
            from = "planner"
            to = ["probe_a", "probe_b"]
            "#,
        );

        assert_eq!(route.from, vec![AgentName::from("planner")]);
        assert_eq!(route.to.len(), 2);
        assert!(!route.is_join());
    }

    #[test]
    fn a_join_is_recognised_with_its_timeout() {
        let route = route(
            r#"
            from = ["probe_a", "probe_b"]
            to = "collector"
            join = "all"
            timeout_sec = 60
            "#,
        );

        assert!(route.is_join());
        assert_eq!(route.join, Some(Join::All));
        assert_eq!(route.timeout_sec, Some(60));
    }

    #[test]
    fn the_only_supported_mode_is_async() {
        let route = route(
            r#"
            from = "a"
            to = "b"
            mode = "async"
            "#,
        );

        assert_eq!(route.mode, Mode::Async);
    }

    #[test]
    fn a_spawn_edge_is_recognised() {
        let route = route(
            r#"
            from = "scanner"
            to = "reviewer"
            mode = "spawn"
            "#,
        );

        assert!(route.is_spawn());
        assert_eq!(route.mode, Mode::Spawn);
    }

    #[test]
    fn a_deferred_mode_is_rejected_with_a_clear_error() {
        let error = toml::from_str::<Route>(
            r#"
            from = "a"
            to = "b"
            mode = "request_response"
            "#,
        )
        .expect_err("request_response was superseded by rendezvous joins");

        assert!(error.to_string().contains("request_response"));
    }

    #[test]
    fn a_route_without_pipelines_is_global() {
        // Every route written before scopes existed has to mean exactly what it meant.
        let route = route(
            r#"
            from = "a"
            to = "b"
            "#,
        );

        assert_eq!(route.pipelines, None);
        assert!(route.applies_to(None));
        assert!(route.applies_to(Some(&PipelineName::new("anything"))));
    }

    #[test]
    fn pipelines_accept_a_string_or_a_list_like_from_and_to() {
        let one = route(
            r#"
            from = "a"
            to = "b"
            pipelines = "eagle-eye"
            "#,
        );
        let many = route(
            r#"
            from = "a"
            to = "b"
            pipelines = ["devforge", "devforge-follow-up"]
            "#,
        );

        assert_eq!(one.pipelines, Some(vec![PipelineName::new("eagle-eye")]));
        assert_eq!(many.pipelines.as_ref().map(Vec::len), Some(2));
    }

    #[test]
    fn a_scoped_route_applies_only_to_its_pipelines_and_never_to_a_chain_with_none() {
        let route = route(
            r#"
            from = "eagle"
            to = "bob"
            pipelines = ["devforge"]
            "#,
        );

        assert!(route.applies_to(Some(&PipelineName::new("devforge"))));
        assert!(!route.applies_to(Some(&PipelineName::new("eagle-eye"))));
        assert!(
            !route.applies_to(None),
            "a chain no pipeline opened has global routes only"
        );
    }

    #[test]
    fn two_routes_overlap_when_some_chain_could_use_both() {
        let global = route("from = \"a\"\nto = \"b\"");
        let devforge = route("from = \"a\"\nto = \"b\"\npipelines = [\"devforge\", \"x\"]");
        let eagle = route("from = \"a\"\nto = \"b\"\npipelines = \"eagle-eye\"");
        let shared = route("from = \"a\"\nto = \"b\"\npipelines = [\"x\"]");

        assert!(global.overlaps(&eagle), "a global route is in every scope");
        assert!(!devforge.overlaps(&eagle));
        assert!(devforge.overlaps(&shared));
        assert_eq!(
            devforge.shared_pipelines(&shared),
            vec![PipelineName::new("x")]
        );
    }
}
