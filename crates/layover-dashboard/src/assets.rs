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
/// Its behaviour, one file per part of the page, each served at its own name. The page loads
/// them in order and `app.js` last, since it starts everything the others define.
const SCRIPTS: &[(&str, &str)] = &[
    // Tracing one agent's routes on the route map.
    ("/routemap.js", include_str!("assets/routemap.js")),
    // Watching sessions: the session list and the read-only terminals.
    ("/sessions.js", include_str!("assets/sessions.js")),
    // Answering help requests, and continuing a chain with its own flags.
    ("/reply.js", include_str!("assets/reply.js")),
    // One chain, whole: its map drawn for it alone, and its runs in order.
    ("/chain.js", include_str!("assets/chain.js")),
    // A chain against the clock, and the flags it ran with.
    ("/timeline.js", include_str!("assets/timeline.js")),
    // What starts next: the queue, scheduled ticks, layovers and skipped ticks.
    ("/upcoming.js", include_str!("assets/upcoming.js")),
    // Triggering a workflow: its flags, its name, and agents to run differently.
    ("/trigger.js", include_str!("assets/trigger.js")),
    // The switch that turns a workflow off and on.
    ("/switch.js", include_str!("assets/switch.js")),
    // The route map: a card per workflow, kept current.
    ("/map.js", include_str!("assets/map.js")),
    // Cost: the windows, day by day, the breakdowns and the Reserve.
    ("/cost.js", include_str!("assets/cost.js")),
    // Addresses for every view and its filters.
    ("/router.js", include_str!("assets/router.js")),
    // The list of chains: filtered, searched and paged.
    ("/chains.js", include_str!("assets/chains.js")),
    // The list of runs, paged.
    ("/runs.js", include_str!("assets/runs.js")),
    // Help requests and learnings.
    ("/journal.js", include_str!("assets/journal.js")),
    // The shared helpers, and starting everything.
    ("/app.js", include_str!("assets/app.js")),
];

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

    let page = Router::new()
        .route("/", get(|| async { html(INDEX) }))
        .route("/style.css", get(|| async { asset("text/css", STYLE) }));
    SCRIPTS
        .iter()
        .fold(page, |page, &(path, body)| {
            page.route(
                path,
                get(move || async move { asset("text/javascript", body) }),
            )
        })
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
