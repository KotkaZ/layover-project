//! Serving MCP over HTTP, and turning a bearer token into an identity.
//!
//! # Why HTTP rather than stdin/stdout
//!
//! MCP's original transport is a pipe to a subprocess the *client* starts. That is exactly
//! backwards here: Layover starts the agent, so the agent cannot start Layover. Every target CLI
//! also speaks the HTTP transport, and one endpoint serving many concurrent runs is both simpler
//! and cheaper than a server process per run.
//!
//! # Why an unknown token is loud
//!
//! Everywhere else in this crate, a refusal is a *successful* response marked `isError`, so the
//! agent reads the reason and carries on. An unknown token is different. It means either a bug in
//! the Tower or a process still calling after its run ended — and in the second case, whatever it
//! is about to do is unaccounted for. There is no itinerary to charge it to and no agent it can
//! be. So it gets an HTTP 401 and no tool ever runs.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::protocol::{self, Request};
use crate::runtime::{Runtime, Session};

/// The header a child sends its run token in.
pub const AUTH_HEADER: &str = "authorization";

/// The path the MCP endpoint is served at.
pub const ENDPOINT_PATH: &str = "/mcp";

/// Where a token becomes an identity.
///
/// A trait rather than a concrete registry so the transport can be tested without a supervisor,
/// and so the only way to obtain a [`Session`] remains "the Tower resolved a token it minted".
pub trait Sessions: Send + Sync {
    /// Who holds this token, if anyone still does.
    fn resolve(&self, token: &str) -> Option<Session>;
}

/// Everything the endpoint needs to answer a call.
#[derive(Clone)]
pub struct Served {
    /// Where tokens are resolved.
    pub sessions: Arc<dyn Sessions>,
    /// What the tools actually do.
    pub runtime: Arc<dyn Runtime + Send + Sync>,
}

/// The MCP endpoint.
pub fn router(served: Served) -> Router {
    Router::new()
        .route(ENDPOINT_PATH, axum::routing::post(call))
        .with_state(served)
}

/// Pulls the token out of an `Authorization: Bearer …` header.
///
/// The `Bearer` prefix is optional because not every CLI adds it, and refusing a token that is
/// otherwise correct over a missing word would be a long afternoon for whoever hit it.
#[must_use]
pub fn token_from(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTH_HEADER)?.to_str().ok()?.trim();

    Some(
        value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))
            .unwrap_or(value)
            .trim(),
    )
}

async fn call(State(served): State<Served>, headers: HeaderMap, body: String) -> Response {
    let Some(session) = token_from(&headers).and_then(|token| served.sessions.resolve(token))
    else {
        // Loud, unlike every other refusal in this crate: an unknown token is a call that cannot
        // be accounted to anything, so no tool may run on it.
        return (
            StatusCode::UNAUTHORIZED,
            "no live run holds that token. Layover mints one per run and revokes it when the run \
             ends.",
        )
            .into_response();
    };

    let request: Request = match serde_json::from_str(&body) {
        Ok(request) => request,
        Err(error) => return as_json(&protocol::malformed(&error.to_string())),
    };

    match protocol::handle(&request, &session, served.runtime.as_ref()) {
        Some(response) => as_json(&response),
        // A notification expects no reply, and sending one is a protocol error at the other end.
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// Serialises a reply, which cannot fail for a type this crate owns.
fn as_json(response: &protocol::Response) -> Response {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(response).unwrap_or_default(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{Peer, ToolError};
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use layover_core::agent::AgentName;
    use layover_core::flight::{ItineraryId, RunId};
    use tower::ServiceExt as _;

    struct OneToken(&'static str, Session);

    impl Sessions for OneToken {
        fn resolve(&self, token: &str) -> Option<Session> {
            (token == self.0).then(|| self.1.clone())
        }
    }

    struct Stub;

    impl Runtime for Stub {
        fn peers(&self, _session: &Session) -> Vec<Peer> {
            vec![Peer {
                name: AgentName::new("developer"),
                description: Some("Writes the code".to_owned()),
                spawns: false,
            }]
        }

        fn send(
            &self,
            _session: &Session,
            _to: &AgentName,
            _body: &str,
        ) -> Result<String, ToolError> {
            Ok("flt_test".to_owned())
        }

        fn report(&self, _: &Session, _: &str, _: &str) -> Result<(), ToolError> {
            Ok(())
        }

        fn help(&self, _: &Session, _: &str, _: &str, _: bool) -> Result<(), ToolError> {
            Ok(())
        }

        fn memory_read(&self, _: &Session) -> Result<String, ToolError> {
            Ok(String::new())
        }

        fn memory_write(&self, _: &Session, _: &str) -> Result<(), ToolError> {
            Ok(())
        }
    }

    fn app() -> Router {
        router(Served {
            sessions: Arc::new(OneToken(
                "lvt_good",
                Session {
                    run: RunId::generate(),
                    agent: AgentName::new("analyst"),
                    itinerary: ItineraryId::generate(),
                    hops_remaining: 3,
                },
            )),
            runtime: Arc::new(Stub),
        })
    }

    async fn post(auth: Option<&str>, body: &str) -> (StatusCode, String) {
        let mut request = HttpRequest::builder()
            .method("POST")
            .uri(ENDPOINT_PATH)
            .header("content-type", "application/json");

        if let Some(auth) = auth {
            request = request.header("authorization", auth);
        }

        let response = app()
            .oneshot(
                request
                    .body(Body::from(body.to_owned()))
                    .expect("a request"),
            )
            .await
            .expect("a response");

        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("a body");

        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn a_token_the_tower_minted_gets_an_answer() {
        let (status, body) = post(
            Some("Bearer lvt_good"),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("layover_send"), "{body}");
    }

    #[tokio::test]
    async fn a_token_nobody_holds_is_refused_before_any_tool_runs() {
        // Not an `isError` result: a call that cannot be accounted to a run must not reach a tool
        // at all.
        let (status, body) = post(
            Some("Bearer lvt_stale"),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(body.contains("revokes it when the run ends"), "{body}");
    }

    #[tokio::test]
    async fn no_token_at_all_is_refused_the_same_way() {
        let (status, _) = post(None, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_bearer_prefix_is_optional() {
        // Not every CLI adds it, and refusing an otherwise correct token over a missing word is a
        // long afternoon for whoever hits it.
        let (status, _) = post(
            Some("lvt_good"),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_notification_gets_no_reply_body() {
        // A JSON-RPC notification has no id and expects no response; sending one is an error at
        // the other end.
        let (status, body) = post(
            Some("Bearer lvt_good"),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        )
        .await;

        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body.is_empty(), "{body}");
    }

    #[tokio::test]
    async fn a_body_that_is_not_json_is_answered_rather_than_dropped() {
        let (status, body) = post(Some("Bearer lvt_good"), "{not json").await;

        assert_eq!(
            status,
            StatusCode::OK,
            "the connection is fine; the body was not"
        );
        assert!(body.contains("error"), "{body}");
    }

    #[tokio::test]
    async fn a_tool_call_reaches_the_runtime() {
        let (status, body) = post(
            Some("Bearer lvt_good"),
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"layover_peers","arguments":{}}}"#,
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("developer"), "{body}");
    }
}
