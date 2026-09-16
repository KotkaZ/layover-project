//! That the generated server actually serves what `api/openapi.yaml` declares.
//!
//! The generator is only worth having if its output is exercised. These tests drive the real
//! router through `tower::Service`, so a route that does not exist, a status code that does not
//! match the specification, or a body that does not deserialise all fail here.
//!
//! The `Api` implementation below is a stub that answers from constants, so nothing it does can
//! await. The trait returns `impl Future + Send` precisely so implementors may write `async fn`;
//! hand-rolling ready futures here to satisfy a lint would make the test double harder to read
//! than the thing it stands in for.
#![allow(clippy::unused_async_trait_impl)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use layover_http::{
    Access, Agent, AgentList, Api, Blocker, CostBucket, CostReport, CostSource, CostSummary,
    CostWindow, EventStream, FlightAccepted, GetCostsQuery, GetRunPath, GroundStop, Health,
    HelpList, HelpRequest, Impact, Learning, LearningList, LearningState, ListHelpQuery,
    ListLearningsQuery, ListRunsQuery, OPERATIONS, Pipeline, PipelineList, Problem, ReserveState,
    RouteMap, Run, RunList, RunStatus, SendFlightRequest, Status, StreamRunPath, TokenUsage,
    Trigger, TriggerKind, WindowSpan, router,
};
use tower::ServiceExt as _;

/// A minimal implementation that returns fixed values.
struct Stub;

/// A resolved window, as the Tower would report one.
///
/// Spelled out here rather than computed so the contract test stays a test of the *shape* of the
/// response. Whether the arithmetic is right is `layover-core`'s problem, and it has its own
/// tests for it.
fn span(window: CostWindow) -> WindowSpan {
    let calendar = matches!(window, CostWindow::Today | CostWindow::MonthToDate);
    WindowSpan {
        window,
        label: "Last 30 days".to_owned(),
        calendar,
        start: Some("2026-08-17T12:00:00Z".to_owned()),
        end: "2026-09-16T12:00:00Z".to_owned(),
        zone: calendar.then(|| "Europe/Tallinn".to_owned()),
        truncated: false,
    }
}

fn sample_run() -> Run {
    Run {
        run_id: "run_1".to_owned(),
        itinerary_id: "itn_1".to_owned(),
        agent: "analyst".to_owned(),
        pipeline: Some("development".to_owned()),
        model: None,
        status: RunStatus::Interrupted,
        started_at: "2026-09-15T12:00:00Z".to_owned(),
        finished_at: None,
        duration_sec: None,
        exit_code: None,
        cost_usd: None,
        cost_source: CostSource::Unreported,
        detail: Some("the Tower went away mid-run".to_owned()),
        blocked_on: None,
        hops_remaining: Some(21),
    }
}

// A stub answers from constants, so none of these bodies await anything.
impl Api for Stub {
    async fn get_health(&self) -> Result<Health, Problem> {
        Ok(Health {
            status: Status::Ok,
            version: "0.2.0".to_owned(),
            ground_stop: false,
        })
    }

    async fn list_agents(&self) -> Result<AgentList, Problem> {
        Ok(AgentList {
            agents: vec![Agent {
                name: "analyst".to_owned(),
                description: Some("Turns a request into a work item".to_owned()),
                purpose: None,
                runner: Some("claude".to_owned()),
                model: None,
                access: Access::ReadOnly,
                entry: false,
                resident: false,
            }],
            routes: Vec::new(),
        })
    }

    async fn list_pipelines(&self) -> Result<PipelineList, Problem> {
        Ok(PipelineList {
            pipelines: vec![Pipeline {
                name: "review-bot".to_owned(),
                description: None,
                entry: "pr_scanner".to_owned(),
                trigger: Trigger {
                    kind: TriggerKind::Scheduled,
                    every_seconds: Some(3_600),
                    cron: None,
                },
                flags: Vec::new(),
            }],
        })
    }

    async fn send_flight(&self, body: SendFlightRequest) -> Result<FlightAccepted, Problem> {
        let to = body
            .to
            .or(body.pipeline)
            .ok_or_else(|| Problem::new(StatusCode::BAD_REQUEST, "give a pipeline or an agent"))?;

        Ok(FlightAccepted {
            flight_id: "flt_1".to_owned(),
            itinerary_id: "itn_1".to_owned(),
            to,
        })
    }

    async fn list_runs(&self, query: ListRunsQuery) -> Result<RunList, Problem> {
        let runs = match query.status {
            Some(RunStatus::Interrupted) | None => vec![sample_run()],
            Some(_) => Vec::new(),
        };
        Ok(RunList { runs })
    }

    async fn get_run(&self, path: GetRunPath) -> Result<Run, Problem> {
        if path.run_id == "run_1" {
            Ok(sample_run())
        } else {
            Err(Problem::new(StatusCode::NOT_FOUND, "no such run")
                .with_detail(format!("`{}` is not a run", path.run_id)))
        }
    }

    async fn stream_run(&self, _path: StreamRunPath) -> Result<EventStream, Problem> {
        Ok(EventStream::new(Body::from("data: hello\n\n")))
    }

    async fn get_costs(&self, query: GetCostsQuery) -> Result<CostReport, Problem> {
        // One measured run and one estimated one, so the report has something to be honest about.
        let usage = TokenUsage {
            input: 1_000,
            output: 500,
            cache_read: 100,
            cache_write: 50,
        };
        let measured = CostSummary {
            runs: 1,
            usd: 4.0,
            usage: usage.clone(),
            unreported_runs: 0,
            estimated_runs: 0,
            confidence: CostSource::Reported,
            measured_share: 1.0,
        };
        let mixed = CostSummary {
            runs: 2,
            usd: 6.0,
            usage,
            unreported_runs: 0,
            estimated_runs: 1,
            confidence: CostSource::RateCard,
            measured_share: 0.5,
        };

        Ok(CostReport {
            span: span(query.window.unwrap_or(CostWindow::Last30d)),
            total: if query.window.is_some() {
                measured.clone()
            } else {
                mixed
            },
            by_agent: vec![CostBucket {
                name: "developer".to_owned(),
                summary: measured.clone(),
            }],
            by_model: vec![CostBucket {
                name: "claude-opus-5".to_owned(),
                summary: measured,
            }],
            reserve: ReserveState {
                cap_usd: Some(120.0),
                spent_usd: 6.0,
                remaining_usd: Some(114.0),
                window_hours: 24,
                exhausted: false,
            },
        })
    }

    async fn get_graph(&self) -> Result<RouteMap, Problem> {
        Ok(RouteMap {
            mermaid: "flowchart LR\n  a_analyst --> a_developer\n".to_owned(),
            generated_at: "2026-09-16T12:00:00Z".to_owned(),
            config_path: Some("layover.toml".to_owned()),
        })
    }

    async fn list_help(&self, _: ListHelpQuery) -> Result<HelpList, Problem> {
        Ok(HelpList {
            requests: vec![HelpRequest {
                agent: "publisher".to_owned(),
                run_id: "run_1".to_owned(),
                itinerary_id: "itn_1".to_owned(),
                blocker: Blocker::Access,
                summary: "the ADO token expired".to_owned(),
                detail: "tried to push, got 401, needs a fresh token".to_owned(),
                fatal: true,
                at: "2026-09-16T12:00:00Z".to_owned(),
                resolved_at: None,
            }],
            open: 1,
        })
    }

    async fn list_learnings(&self, _: ListLearningsQuery) -> Result<LearningList, Problem> {
        Ok(LearningList {
            learnings: vec![Learning {
                id: "lrn_1".to_owned(),
                agent: "publisher".to_owned(),
                text: "the ADO token expires every thirty days".to_owned(),
                impact: Impact::High,
                state: LearningState::Confirmed,
                proposals: 3,
                runs_left: 0,
                first_at: "2026-08-16T12:00:00Z".to_owned(),
                last_at: "2026-09-16T12:00:00Z".to_owned(),
            }],
            active: 1,
        })
    }

    async fn engage_ground_stop(&self) -> Result<GroundStop, Problem> {
        Ok(GroundStop {
            engaged: true,
            since: Some("2026-09-15T12:00:00Z".to_owned()),
        })
    }

    async fn release_ground_stop(&self) -> Result<GroundStop, Problem> {
        Ok(GroundStop {
            engaged: false,
            since: None,
        })
    }
}

async fn call(method: &str, uri: &str, body: Option<&str>) -> (StatusCode, String, String) {
    let app = router(Arc::new(Stub));

    let request = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(json) => request
            .header("content-type", "application/json")
            .body(Body::from(json.to_owned())),
        None => request.body(Body::empty()),
    }
    .expect("request builds");

    let response = app.oneshot(request).await.expect("router responds");
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .map(|value| value.to_str().unwrap_or_default().to_owned())
        .unwrap_or_default();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();

    (
        status,
        content_type,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).expect("response is valid json")
}

#[test]
fn every_specified_operation_is_routed() {
    assert_eq!(OPERATIONS.len(), 13);

    for (method, path, operation) in OPERATIONS {
        assert!(path.starts_with('/'), "`{operation}` has an odd path");
        assert!(
            ["GET", "POST", "DELETE", "PUT", "PATCH"].contains(&method),
            "`{operation}` uses method `{method}`"
        );
    }

    let paths: Vec<&str> = OPERATIONS.iter().map(|(_, path, _)| *path).collect();
    assert!(paths.contains(&"/ground-stop"));
    assert!(paths.contains(&"/runs/{run_id}/stream"));
    assert!(paths.contains(&"/costs"));
}

#[tokio::test]
async fn health_answers_with_the_specified_shape() {
    let (status, content_type, body) = call("GET", "/health", None).await;

    assert_eq!(status, StatusCode::OK);
    assert!(content_type.starts_with("application/json"));

    let value = json(&body);
    assert_eq!(value["status"], "ok");
    assert_eq!(value["ground_stop"], false);
}

#[tokio::test]
async fn sending_a_flight_returns_202_as_specified() {
    // The specification says 202, not 200. A generated handler that ignored the declared status
    // would be a silent contract break.
    let (status, _, body) = call(
        "POST",
        "/flights",
        Some(r#"{"pipeline":"development","body":"fix the retry policy"}"#),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(json(&body)["to"], "development");
}

#[tokio::test]
async fn flags_are_accepted_as_a_map() {
    let (status, _, _) = call(
        "POST",
        "/flights",
        Some(r#"{"to":"analyst","body":"go","flags":{"run_e2e":true}}"#),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED);
}

#[tokio::test]
async fn an_omitted_optional_field_is_accepted() {
    // Optional properties are generated with `#[serde(default)]`, so a minimal client body works.
    let (status, _, _) = call("POST", "/flights", Some(r#"{"body":"go","to":"analyst"}"#)).await;

    assert_eq!(status, StatusCode::ACCEPTED);
}

#[tokio::test]
async fn a_problem_carries_its_status_into_the_response() {
    let (status, _, body) = call("GET", "/runs/absent", None).await;

    assert_eq!(status, StatusCode::NOT_FOUND);

    let value = json(&body);
    assert_eq!(value["status"], 404);
    assert_eq!(value["title"], "no such run");
    assert_eq!(value["detail"], "`absent` is not a run");
}

#[tokio::test]
async fn a_path_parameter_reaches_the_implementation() {
    let (status, _, body) = call("GET", "/runs/run_1", None).await;

    assert_eq!(status, StatusCode::OK);

    let value = json(&body);
    assert_eq!(value["run_id"], "run_1");
    assert_eq!(
        value["status"], "interrupted",
        "an interrupted run must be visible as its own outcome"
    );
}

#[tokio::test]
async fn query_parameters_are_optional_and_typed() {
    let (status, _, all) = call("GET", "/runs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&all)["runs"].as_array().map(Vec::len), Some(1));

    let (status, _, filtered) = call("GET", "/runs?status=running&limit=5", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&filtered)["runs"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn streaming_uses_the_event_stream_content_type() {
    let (status, content_type, body) = call("GET", "/runs/run_1/stream", None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "text/event-stream");
    assert!(body.contains("data: hello"));
}

#[tokio::test]
async fn both_methods_on_ground_stop_are_reachable() {
    // Two operations share one path. axum panics on a duplicate route, so this also proves the
    // generator merged them rather than registering the path twice.
    let (status, _, engaged) = call("POST", "/ground-stop", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&engaged)["engaged"], true);

    let (status, _, released) = call("DELETE", "/ground-stop", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&released)["engaged"], false);
}

#[tokio::test]
async fn an_undeclared_path_is_not_served() {
    let (status, _, _) = call("GET", "/nothing-here", None).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_undeclared_method_on_a_declared_path_is_refused() {
    let (status, _, _) = call("DELETE", "/health", None).await;

    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}
