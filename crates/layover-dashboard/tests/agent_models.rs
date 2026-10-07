//! What the dashboard says about each agent: the model it runs on, and its routes when traced.

use std::fs;
use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use layover_dashboard::{Dashboard, DashboardState, Guard, router};
use layover_store::{History, Journal};
use tower::ServiceExt as _;

struct Factory(PathBuf);

impl Factory {
    fn new(label: &str, config: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "layover-dash-models-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("history")).expect("temp dirs");
        fs::write(root.join("layover.toml"), config).expect("writes config");
        Self(root)
    }

    async fn get(&self, path: &str) -> (StatusCode, serde_json::Value) {
        let (status, body) = self.text(path).await;
        (status, serde_json::from_str(&body).expect("a JSON body"))
    }

    async fn text(&self, path: &str) -> (StatusCode, String) {
        let app = router(
            Dashboard::new(DashboardState {
                config_path: self.0.join("layover.toml"),
                history: History::open(self.0.join("history")).expect("opens history"),
                journal: std::sync::Arc::new(
                    Journal::open(self.0.join("journal")).expect("opens journal"),
                ),
                ground_stop: self.0.join("ground-stop"),
            }),
            Guard::Open,
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("responds");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const FACTORY: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "shadow"

[runners.shadow]
command = ["copilot", "--model", "claude-opus-5.5", "--reasoning-effort", "xhigh", "--context", "long_context"]

[runners.claude]
command = ["claude", "-p", "--model", "{model}"]

[runners.analysis]
command = ["copilot", "--model", "{model}", "--reasoning-effort={effort}", "--context={context}", "--allow-all-tools"]

[agents.analyst]
prompt = "analyse"

[agents.bob]
prompt = "build"
runner = "claude"
model = "claude-sonnet-5"

[agents.eagle]
prompt = "review"
runner = "analysis"
model = "claude-opus-5.5"
effort = "xhigh"
context = "long_context"

[agents.tars]
prompt = "triage"
runner = "analysis"
model = "claude-opus-5.5"
effort = "high"

[pipelines.devforge]
entry = "analyst"

[[routes]]
from = "analyst"
to = ["bob", "eagle", "tars"]
"#;

fn agent<'a>(body: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    body["agents"]
        .as_array()
        .and_then(|agents| agents.iter().find(|agent| agent["name"] == name))
        .expect("the agent is listed")
}

#[tokio::test]
async fn an_agent_reports_the_model_effort_and_context_its_command_sets() {
    let factory = Factory::new("command", FACTORY);
    let (status, body) = factory.get("/agents").await;

    assert_eq!(status, StatusCode::OK);
    let analyst = agent(&body, "analyst");
    assert_eq!(analyst["model"], "claude-opus-5.5", "{analyst}");
    assert_eq!(analyst["reasoning_effort"], "xhigh", "{analyst}");
    assert_eq!(analyst["context"], "long_context", "{analyst}");
}

#[tokio::test]
async fn an_agent_reports_the_model_it_declares_and_nothing_it_does_not_set() {
    let factory = Factory::new("declared", FACTORY);
    let (_, body) = factory.get("/agents").await;

    let bob = agent(&body, "bob");
    assert_eq!(bob["model"], "claude-sonnet-5", "{bob}");
    assert!(bob["reasoning_effort"].is_null(), "{bob}");
    assert!(bob["context"].is_null(), "{bob}");
}

#[tokio::test]
async fn a_workflows_map_names_each_agents_model() {
    let factory = Factory::new("map", FACTORY);
    let (_, body) = factory.get("/graph?pipeline=devforge").await;
    let svg = body["mermaid"].as_str().expect("a drawing");

    assert!(
        svg.contains(">claude-opus-5.5 · effort xhigh</text>"),
        "{svg}"
    );
    assert!(svg.contains(">claude-sonnet-5</text>"), "{svg}");
}

#[tokio::test]
async fn agents_sharing_a_runner_report_their_own_effort_and_context() {
    // One runner, one permission set; the effort and context belong to each agent.
    let factory = Factory::new("shared", FACTORY);
    let (_, body) = factory.get("/agents").await;

    let eagle = agent(&body, "eagle");
    assert_eq!(eagle["model"], "claude-opus-5.5", "{eagle}");
    assert_eq!(eagle["reasoning_effort"], "xhigh", "{eagle}");
    assert_eq!(eagle["context"], "long_context", "{eagle}");

    let tars = agent(&body, "tars");
    assert_eq!(tars["reasoning_effort"], "high", "{tars}");
    assert!(
        tars["context"].is_null(),
        "it sets none, so its runner passes none: {tars}"
    );
}

#[tokio::test]
async fn the_map_draws_each_agent_on_a_shared_runner_at_its_own_effort() {
    let factory = Factory::new("shared-map", FACTORY);
    let (_, body) = factory.get("/graph?pipeline=devforge").await;
    let svg = body["mermaid"].as_str().expect("a drawing");

    let node = |id: &str| {
        svg.split(&format!(r#"id="{id}""#))
            .nth(1)
            .and_then(|rest| rest.split("</g>").next())
            .unwrap_or_else(|| panic!("{id} is drawn: {svg}"))
            .to_owned()
    };
    let eagle = node("a_eagle");
    assert!(
        eagle.contains(">claude-opus-5.5 · effort xhigh</text>"),
        "{eagle}"
    );
    assert!(eagle.contains(">long context</text>"), "{eagle}");
    let tars = node("a_tars");
    assert!(
        tars.contains(">claude-opus-5.5 · effort high</text>"),
        "{tars}"
    );
    assert!(!tars.contains("long context"), "{tars}");
}

#[tokio::test]
async fn a_run_says_which_effort_and_context_it_ran_at() {
    let factory = Factory::new("run-effort", FACTORY);
    let mut record = layover_core::run::RunRecord::started(
        layover_core::RunId::generate(),
        layover_core::flight::ItineraryId::generate(),
        "eagle".into(),
        jiff::Timestamp::now(),
    )
    .finished(
        layover_core::run::Outcome::Succeeded,
        jiff::Timestamp::now(),
    );
    record.model = Some("claude-opus-5.5".to_owned());
    record.effort = Some("xhigh".to_owned());
    record.context = Some("long_context".to_owned());
    History::open(factory.0.join("history"))
        .expect("opens history")
        .append(&record)
        .expect("records");

    let (_, body) = factory.get("/runs").await;
    let run = &body["runs"][0];

    assert_eq!(run["model"], "claude-opus-5.5", "{run}");
    assert_eq!(run["reasoning_effort"], "xhigh", "{run}");
    assert_eq!(run["context"], "long_context", "{run}");
}

#[tokio::test]
async fn the_page_can_trace_an_agents_routes() {
    // Hovering an agent lights its routes and clicking pins them, which is what makes a busy map
    // readable one agent at a time. The script ships in the binary like everything else.
    let factory = Factory::new("trace", FACTORY);

    let (status, script) = factory.text("/routemap.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(script.contains("function traceable"), "{script}");

    let (_, page) = factory.text("/").await;
    let map = page.find(r#"<script src="/routemap.js">"#);
    let app = page.find(r#"<script src="/app.js">"#);
    assert!(
        map.zip(app).is_some_and(|(map, app)| map < app),
        "the tracing script has to load before the page that uses it"
    );
}
