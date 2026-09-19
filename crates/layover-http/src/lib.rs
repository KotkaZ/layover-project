//! The Layover Tower HTTP surface.
//!
//! Everything in [`generated`] comes from `api/openapi.yaml` by way of
//! `cargo xtask generate-api`. The specification is the contract: `cargo xtask verify`
//! regenerates the module and fails if it differs, so the server cannot drift away from the
//! document that describes it.
//!
//! This crate provides the *shape* of the API and nothing behind it. Implement [`Api`] to serve
//! it; the Tower will, once it exists.
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use layover_http::{Api, Health, Problem, Status, router};
//! struct Stub;
//!
//! impl Api for Stub {
//!     async fn get_health(&self) -> Result<Health, Problem> {
//!         Ok(Health {
//!             status: Status::Ok,
//!             version: env!("CARGO_PKG_VERSION").to_owned(),
//!             ground_stop: false,
//!         })
//!     }
//!     # async fn list_agents(&self) -> Result<layover_http::AgentList, Problem> { todo!() }
//!     # async fn list_pipelines(&self) -> Result<layover_http::PipelineList, Problem> { todo!() }
//!     # async fn get_graph(&self, _: layover_http::GetGraphQuery) -> Result<layover_http::RouteMap, Problem> { todo!() }
//!     # async fn list_help(&self, _: layover_http::ListHelpQuery) -> Result<layover_http::HelpList, Problem> { todo!() }
//!     # async fn list_learnings(&self, _: layover_http::ListLearningsQuery) -> Result<layover_http::LearningList, Problem> { todo!() }
//!     # async fn get_costs(&self, _: layover_http::GetCostsQuery) -> Result<layover_http::CostReport, Problem> { todo!() }
//!     # async fn send_flight(&self, _: layover_http::SendFlightRequest) -> Result<layover_http::FlightAccepted, Problem> { todo!() }
//!     # async fn list_runs(&self, _: layover_http::ListRunsQuery) -> Result<layover_http::RunList, Problem> { todo!() }
//!     # async fn get_run(&self, _: layover_http::GetRunPath) -> Result<layover_http::Run, Problem> { todo!() }
//!     # async fn stream_run(&self, _: layover_http::StreamRunPath) -> Result<layover_http::EventStream, Problem> { todo!() }
//!     # async fn list_pending(&self) -> Result<layover_http::PendingList, Problem> { todo!() }
//!     # async fn resolve_help(&self, _: layover_http::ResolveHelpRequest) -> Result<layover_http::HelpResolved, Problem> { todo!() }
//!     # async fn judge_learning(&self, _: layover_http::JudgeLearningPath, _: layover_http::JudgeLearningRequest) -> Result<layover_http::Learning, Problem> { todo!() }
//!     # async fn cancel_flight(&self, _: layover_http::CancelFlightPath) -> Result<layover_http::PendingList, Problem> { todo!() }
//!     # async fn list_itineraries(&self, _: layover_http::ListItinerariesQuery) -> Result<layover_http::ItineraryList, Problem> { todo!() }
//!     # async fn get_report(&self, _: layover_http::GetReportPath) -> Result<layover_http::Report, Problem> { todo!() }
//!     # async fn engage_ground_stop(&self) -> Result<layover_http::GroundStop, Problem> { todo!() }
//!     # async fn release_ground_stop(&self) -> Result<layover_http::GroundStop, Problem> { todo!() }
//! }
//!
//! let app = router(Arc::new(Stub));
//! ```

pub mod generated;

pub use generated::*;

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

/// A server-sent event stream, returned by streaming operations.
///
/// Deliberately a thin wrapper over a body rather than a concrete stream type: the Tower decides
/// how it produces events, and this crate only promises the content type the specification
/// declares.
pub struct EventStream(axum::body::Body);

impl EventStream {
    /// Wraps a body as an event stream.
    #[must_use]
    pub fn new(body: axum::body::Body) -> Self {
        Self(body)
    }
}

impl IntoResponse for EventStream {
    fn into_response(self) -> Response {
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/event-stream"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            self.0,
        )
            .into_response()
    }
}

impl Problem {
    /// Builds a problem with a status and title.
    #[must_use]
    pub fn new(status: StatusCode, title: impl Into<String>) -> Self {
        Self {
            status: i32::from(status.as_u16()),
            title: title.into(),
            detail: None,
        }
    }

    /// Adds explanatory detail.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Returns the HTTP status this problem carries.
    ///
    /// Falls back to `500` when the numeric status is not a valid HTTP code, because a malformed
    /// error must still produce a response rather than panicking inside a handler.
    #[must_use]
    pub fn status_code(&self) -> StatusCode {
        u16::try_from(self.status)
            .ok()
            .and_then(|code| StatusCode::from_u16(code).ok())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        (self.status_code(), axum::Json(self)).into_response()
    }
}
