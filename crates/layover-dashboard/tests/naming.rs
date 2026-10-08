//! A chain's name and the per-agent choices made when triggering it: accepted only when they can
//! take effect, kept with the queued work, and shown wherever the chain is.

use std::fs;
use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use layover_dashboard::{Dashboard, DashboardState, Guard, router};
use layover_store::{History, Journal};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const FACTORY: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "copilot"

[runners.copilot]
cli = "copilot"

[runners.script]
command = ["python", "notify.py"]

[agents.analyst]
prompt = "analyse"
model = "claude-opus-5.5"
effort = "xhigh"

[agents.reviewer]
prompt = "review"
model = "claude-opus-5.5"
effort = "xhigh"
context = "long_context"

[agents.notifier]
prompt = "notify"
runner = "script"

[agents.outsider]
prompt = "elsewhere"
entry = true

[pipelines.development]
entry = "analyst"

[pipelines.development.agents.reviewer]
effort = "high"

[[routes]]
from = "analyst"
to = ["reviewer", "notifier"]
"#;

struct Factory {
    root: PathBuf,
}

impl Factory {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "layover-dash-naming-{label}-{}",
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

    async fn trigger(&self, extra: Value) -> (StatusCode, Value) {
        let mut body = json!({ "pipeline": "development", "body": "Build the retry banner" });
        if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
            body.extend(extra.clone());
        }
        self.call("POST", "/flights", Some(body)).await
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn a_named_trigger_is_queued_with_its_name_and_choices_and_listed_by_name() {
    let factory = Factory::new("named");

    let (status, accepted) = factory
        .trigger(json!({
            "name": "  Login page: retry banner ",
            "agents": { "reviewer": { "effort": "max" }, "analyst": {} }
        }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");

    let queued = factory.journal().pending().expect("reads");
    assert_eq!(queued.len(), 1);
    let chosen = &queued[0].chosen;
    assert_eq!(chosen.name.as_deref(), Some("Login page: retry banner"));
    assert_eq!(
        chosen
            .agents
            .keys()
            .map(layover_core::agent::AgentName::as_str)
            .collect::<Vec<_>>(),
        ["reviewer"],
        "an empty choice is not a choice"
    );

    let (_, flights) = factory.call("GET", "/flights", None).await;
    assert_eq!(flights["pending"][0]["name"], "Login page: retry banner");

    let (_, chains) = factory.call("GET", "/itineraries", None).await;
    let chain = chains["itineraries"]
        .as_array()
        .and_then(|chains| {
            chains
                .iter()
                .find(|chain| chain["itinerary_id"] == accepted["itinerary_id"])
        })
        .expect("listed");
    assert_eq!(chain["name"], "Login page: retry banner");
}

#[tokio::test]
async fn a_choice_that_could_not_take_effect_is_refused_rather_than_ignored() {
    let factory = Factory::new("refused");

    for (extra, says) in [
        (
            json!({ "agents": { "outsider": { "effort": "high" } } }),
            "never runs in this workflow",
        ),
        (
            json!({ "agents": { "notifier": { "effort": "high" } } }),
            "no `{effort}` placeholder",
        ),
        (
            json!({ "agents": { "reviewer": { "effort": "high --yolo" } } }),
            "letters, digits",
        ),
        (
            json!({ "agents": { "ghost": { "model": "m" } } }),
            "not an agent",
        ),
        (json!({ "name": "two\nlines" }), "one line"),
    ] {
        let (status, problem) = factory.trigger(extra.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{extra}: {problem}");
        let detail = problem["detail"].as_str().unwrap_or_default();
        assert!(detail.contains(says), "{extra}: {detail}");
    }
    assert!(
        factory.journal().pending().expect("reads").is_empty(),
        "nothing refused was queued"
    );
}

#[tokio::test]
async fn a_pipeline_says_which_agents_a_trigger_may_choose_for_and_what_they_run_at_now() {
    let factory = Factory::new("pipeline-agents");

    let (_, body) = factory.call("GET", "/pipelines", None).await;
    let agents = body["pipelines"][0]["agents"].as_array().expect("a list");
    let names: Vec<&str> = agents.iter().filter_map(|a| a["agent"].as_str()).collect();
    assert_eq!(names, ["analyst", "notifier", "reviewer"], "entry first");

    let reviewer = &agents[2];
    assert_eq!(
        reviewer["reasoning_effort"], "high",
        "the workflow's override"
    );
    assert_eq!(reviewer["context"], "long_context");
    assert_eq!(reviewer["takes_effort"], true);

    let notifier = &agents[1];
    assert_eq!(
        notifier["takes_effort"], false,
        "its runner has no `{{effort}}`"
    );
}

#[tokio::test]
async fn a_chains_map_shows_what_its_agents_ran_at_in_it() {
    // The workflow's map says what the factory declares; one chain's says what that chain ran.
    let factory = Factory::new("chain-map");
    let chain = layover_core::flight::ItineraryId::generate();
    let mut record = layover_core::run::RunRecord::started(
        layover_core::RunId::generate(),
        chain.clone(),
        "analyst".into(),
        jiff::Timestamp::now(),
    )
    .from_pipeline(layover_core::pipeline::PipelineName::new("development"))
    .finished(
        layover_core::run::Outcome::Succeeded,
        jiff::Timestamp::now(),
    );
    record.model = Some("claude-opus-5.5".to_owned());
    record.effort = Some("max".to_owned());
    record.chain_name = Some("Retry banner".to_owned());
    History::open(factory.root.join(".layover").join("history"))
        .expect("opens")
        .append(&record)
        .expect("records");

    let (_, detail) = factory
        .call("GET", &format!("/itineraries/{}", chain.as_str()), None)
        .await;
    let svg = detail["map"]["mermaid"].as_str().expect("a drawing");
    assert!(
        svg.contains(">claude-opus-5.5 · effort max</text>"),
        "{svg}"
    );
    assert_eq!(detail["itinerary"]["name"], "Retry banner");

    let (_, workflow) = factory
        .call("GET", "/graph?pipeline=development", None)
        .await;
    let svg = workflow["mermaid"].as_str().expect("a drawing");
    assert!(
        svg.contains(">claude-opus-5.5 · effort xhigh</text>"),
        "{svg}"
    );
}
