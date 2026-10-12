//! Switching a workflow off and on over HTTP: a trigger is refused while it is off, one queued
//! before is held and says so, and a chain already under way is not.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use layover_core::agent::AgentName;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_dashboard::{Dashboard, DashboardState, Guard, router};
use layover_store::{History, Journal};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const FACTORY: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "claude"

[runners.claude]
cli = "claude"

[agents.analyst]
prompt = "analyse"

[agents.reviewer]
prompt = "review"

[agents.outsider]
prompt = "elsewhere"
entry = true

[pipelines.development]
entry = "analyst"

[pipelines.sweep]
entry = "reviewer"
trigger = { every = "2h" }

[[routes]]
from = "analyst"
to = "reviewer"
"#;

struct Factory {
    root: PathBuf,
}

impl Factory {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "layover-dash-switch-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".layover").join("history")).expect("temp dirs");
        fs::write(root.join("layover.toml"), FACTORY).expect("writes config");
        Self { root }
    }

    fn journal(&self) -> Journal {
        Journal::open(self.root.join(".layover").join("journal")).expect("opens")
    }

    fn router(&self) -> Router {
        let layover = self.root.join(".layover");
        router(
            Dashboard::new(DashboardState {
                config_path: self.root.join("layover.toml"),
                history: History::open(layover.join("history")).expect("opens history"),
                journal: std::sync::Arc::new(self.journal()),
                ground_stop: layover.join("ground-stop"),
            }),
            Guard::Open,
        )
    }

    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let request = Request::builder().method(method).uri(path);
        let request = match body {
            Some(body) => request
                .header("content-type", "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .expect("request builds");
        let response = self.router().oneshot(request).await.expect("responds");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn trigger(&self, body: Value) -> (StatusCode, Value) {
        self.call("POST", "/flights", Some(body)).await
    }

    async fn pipeline(&self, name: &str) -> Value {
        let (_, list) = self.call("GET", "/pipelines", None).await;
        list["pipelines"]
            .as_array()
            .and_then(|all| all.iter().find(|p| p["name"] == name).cloned())
            .unwrap_or_else(|| panic!("{name} is listed: {list}"))
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn development() -> Value {
    json!({ "pipeline": "development", "body": "Build the retry banner" })
}

#[tokio::test]
async fn a_switched_off_workflow_refuses_a_trigger_until_it_is_switched_back_on() {
    let factory = Factory::new("refuses");

    let (status, off) = factory
        .call("POST", "/pipelines/development/switch-off", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{off}");
    assert_eq!(off["switched_off"], true);
    assert!(off["since"].is_string(), "{off}");

    let listed = factory.pipeline("development").await;
    assert_eq!(listed["switched_off"], true);
    assert_eq!(listed["switched_off_since"], off["since"]);
    assert_eq!(factory.pipeline("sweep").await["switched_off"], false);

    let (status, refused) = factory.trigger(development()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["title"], "that workflow is switched off");
    assert!(factory.journal().pending().expect("reads").is_empty());

    // An entry agent is not a workflow, so no workflow's switch stands in its way.
    let (status, _) = factory
        .trigger(json!({ "to": "outsider", "body": "look" }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, on) = factory
        .call("DELETE", "/pipelines/development/switch-off", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{on}");
    assert_eq!(on["switched_off"], false);
    assert!(on["since"].is_null());
    assert_eq!(factory.pipeline("development").await["switched_off"], false);

    let (status, accepted) = factory.trigger(development()).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
}

#[tokio::test]
async fn switching_a_workflow_that_is_not_declared_is_not_found() {
    let factory = Factory::new("unknown");

    for method in ["POST", "DELETE"] {
        let (status, problem) = factory
            .call(method, "/pipelines/nowhere/switch-off", None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}: {problem}");
    }
    assert!(factory.journal().switched_off().expect("reads").is_empty());
}

#[tokio::test]
async fn a_trigger_queued_before_the_switch_is_held_and_a_chain_under_way_is_not() {
    let factory = Factory::new("held");
    let (status, accepted) = factory.trigger(development()).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // A hand-off of a development chain that is already running.
    factory
        .journal()
        .queue(Queued::new(
            Flight::new(
                ItineraryId::generate(),
                Origin::Agent(AgentName::new("analyst")),
                AgentName::new("reviewer"),
                "review this",
                6,
            ),
            Some(PipelineName::new("development")),
            BTreeMap::new(),
        ))
        .expect("queues");

    factory
        .call("POST", "/pipelines/development/switch-off", None)
        .await;

    let (_, flights) = factory.call("GET", "/flights", None).await;
    let held: Vec<(Value, Value)> = flights["pending"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|flight| (flight["to"].clone(), flight["held"].clone()))
        .collect();
    assert_eq!(
        held,
        [
            (json!("analyst"), json!(true)),
            (json!("reviewer"), json!(false))
        ]
    );

    let (_, chains) = factory.call("GET", "/itineraries", None).await;
    let chain = chains["itineraries"]
        .as_array()
        .and_then(|chains| {
            chains
                .iter()
                .find(|chain| chain["itinerary_id"] == accepted["itinerary_id"])
        })
        .expect("listed");
    assert_eq!(chain["state"], "halted", "{chain}");
    assert!(
        chain["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("switched off")),
        "{chain}"
    );
}
