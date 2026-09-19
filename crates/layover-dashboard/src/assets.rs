//! Serving the dashboard's static assets.
//!
//! Embedded with `include_str!` rather than read from disk: the binary is installed on its own,
//! with no directory of files beside it, and a dashboard that only works from a source checkout
//! is not one anybody would find.

use std::sync::Arc;

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::api::Dashboard;
use crate::auth::Guard;

/// The page.
const INDEX: &str = include_str!("assets/index.html");
/// Its styles.
const STYLE: &str = include_str!("assets/style.css");
/// Its behaviour.
const SCRIPT: &str = include_str!("assets/app.js");

/// Builds the complete server: the JSON API, plus the dashboard on top of it.
///
/// The page is layered over the generated router rather than built into it, which keeps the
/// specification the single source of truth for the API while leaving the UI free to change
/// without regenerating anything.
///
/// Everything is behind the guard, including the page and its assets. Serving the page without a
/// token and letting its first API call fail would look like a broken dashboard rather than a
/// closed door, and the person seeing it would have no idea what to do.
pub fn router(dashboard: Dashboard, guard: Guard) -> Router {
    let api = layover_http::router(Arc::new(dashboard));

    Router::new()
        .route("/", get(|| async { html(INDEX) }))
        .route("/style.css", get(|| async { asset("text/css", STYLE) }))
        .route(
            "/app.js",
            get(|| async { asset("text/javascript", SCRIPT) }),
        )
        .merge(api)
        .layer(axum::middleware::from_fn_with_state(
            guard,
            crate::auth::require,
        ))
}

/// Serves the page.
fn html(body: &'static str) -> Response {
    asset("text/html; charset=utf-8", body)
}

/// Serves a static asset.
///
/// Explicitly uncached. The assets change only when the binary does, but a stale dashboard that
/// disagrees with the API it is talking to is a confusing thing to debug, and this is a local
/// server where a few kilobytes per reload costs nothing.
fn asset(content_type: &'static str, body: &'static str) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}
