//! Edges in the route map.
//!
//! An edge is a *permission*, not a step in a pipeline: it says agent A may send to agent B, and
//! nothing about when or whether it will. Rendezvous joins are the one qualification, and they
//! constrain the receiving node rather than the sender — see [`crate::barrier`].

use serde::{Deserialize, Serialize};

use crate::agent::AgentName;

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
    /// Fire-and-forget. The sender continues immediately.
    #[default]
    Async,
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
}

impl Route {
    /// Returns `true` when this edge parks flights at a barrier.
    #[must_use]
    pub fn is_join(&self) -> bool {
        self.join.is_some()
    }
}

/// Accepts either `from = "a"` or `from = ["a", "b"]`.
fn one_or_many<'de, D>(deserializer: D) -> Result<Vec<AgentName>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        One(AgentName),
        Many(Vec<AgentName>),
    }

    Ok(match Raw::deserialize(deserializer)? {
        Raw::One(name) => vec![name],
        Raw::Many(names) => names,
    })
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
}
