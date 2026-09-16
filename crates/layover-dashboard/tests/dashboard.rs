//! What the dashboard serves, end to end.
//!
//! Against a real configuration file and a real history directory, because the whole point of the
//! dashboard is that it reads both from disk on every request. A test with the config in memory
//! would not catch the thing most worth catching: that editing `layover.toml` and reloading
//! actually changes what you see.

use std::fs;
use std::path::{Path, PathBuf};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use layover_dashboard::{Dashboard, DashboardState, router};
use layover_store::{History, Journal};
use tower::ServiceExt as _;

/// A temporary factory: a configuration file and a history directory that clean themselves up.
struct Factory(PathBuf);

impl Factory {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "layover-dash-{label}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock is after 1970")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("history")).expect("temp dirs");
        let factory = Self(root);
        factory.write_config(TWO_AGENTS);
        factory
    }

    fn write_config(&self, body: &str) {
        fs::write(self.0.join("layover.toml"), body).expect("writes config");
    }

    fn write_run(&self, line: &str) {
        let path = self.0.join("history").join("runs-2026-09-16.jsonl");
        let mut existing = fs::read_to_string(&path).unwrap_or_default();
        existing.push_str(line);
        existing.push('\n');
        fs::write(path, existing).expect("writes history");
    }

    fn router(&self) -> Router {
        router(Dashboard::new(DashboardState {
            config_path: self.0.join("layover.toml"),
            history: History::open(self.0.join("history")).expect("opens history"),
            journal: Journal::open(self.0.join("journal")).expect("opens journal"),
        }))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const TWO_AGENTS: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "claude"

[runners.claude]
command = ["claude", "-p"]

[agents.analyst]
prompt = "analyse"

[agents.developer]
prompt = "develop"

[pipelines.triage]
entry = "analyst"

[[routes]]
from = "analyst"
to = "developer"
"#;

async fn call(app: Router, path: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();

    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).expect("response is valid json")
}

#[tokio::test]
async fn the_page_and_its_assets_are_served_from_the_binary() {
    // Embedded rather than read from disk: the binary is installed on its own, so a dashboard
    // that needed a directory of files beside it would only work from a source checkout.
    let factory = Factory::new("assets");

    for (path, needle) in [
        ("/", "<title>Layover</title>"),
        ("/style.css", ".routemap"),
        ("/app.js", "async function loadCost"),
    ] {
        let (status, body) = call(factory.router(), path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(body.contains(needle), "{path} is missing {needle}");
    }
}

#[tokio::test]
async fn the_route_map_is_drawn_from_the_file_on_disk() {
    let factory = Factory::new("graph");

    let (status, body) = call(factory.router(), "/graph").await;

    assert_eq!(status, StatusCode::OK);
    let value = json(&body);
    let svg = value["mermaid"].as_str().expect("svg source");
    assert!(svg.starts_with("<svg"), "{svg}");
    assert!(svg.contains(r#"id="a_analyst""#));
    assert!(svg.contains(r#"id="p_triage""#));
}

#[tokio::test]
async fn editing_the_configuration_changes_the_diagram_without_a_restart() {
    // This is the difference between a dashboard and a snapshot, and the reason the config is
    // re-read per request rather than cached at startup.
    let factory = Factory::new("reload");
    let (_, before) = call(factory.router(), "/graph").await;
    assert!(
        !json(&before)["mermaid"]
            .as_str()
            .unwrap()
            .contains("a_auditor")
    );

    factory.write_config(&format!(
        "{TWO_AGENTS}\n[agents.auditor]\nprompt = \"audit\"\n"
    ));

    let (_, after) = call(factory.router(), "/graph").await;
    assert!(
        json(&after)["mermaid"]
            .as_str()
            .unwrap()
            .contains("a_auditor"),
        "the new agent should appear on a plain reload"
    );
}

#[tokio::test]
async fn a_broken_configuration_is_reported_rather_than_served_as_an_empty_graph() {
    // An empty diagram looks like a factory with nothing in it, which is a very misleading way
    // to be told the file has a syntax error.
    let factory = Factory::new("broken");
    factory.write_config("this is not toml {{{");

    let (status, body) = call(factory.router(), "/graph").await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        json(&body)["title"]
            .as_str()
            .unwrap()
            .contains("could not be read")
    );
}

#[tokio::test]
async fn history_reaches_the_runs_endpoint() {
    let factory = Factory::new("runs");
    factory.write_run(
        r#"{"run":"run_a","itinerary":"itn_1","agent":"developer","pipeline":"triage","outcome":"succeeded","started_at":"2026-09-16T10:00:00Z","finished_at":"2026-09-16T10:02:00Z","usd":1.5,"source":"reported"}"#,
    );

    let (status, body) = call(factory.router(), "/runs?window=all_time").await;

    assert_eq!(status, StatusCode::OK);
    let runs = json(&body)["runs"].as_array().expect("runs").clone();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["agent"], "developer");
    assert_eq!(runs[0]["status"], "succeeded");
    assert_eq!(runs[0]["duration_sec"], 120);
    assert_eq!(runs[0]["cost_usd"], 1.5);
}

#[tokio::test]
async fn an_unreported_cost_is_null_rather_than_zero() {
    // A zero is indistinguishable from a run that genuinely cost nothing, and the difference is
    // what decides whether the budget rail is working at all.
    let factory = Factory::new("unreported");
    factory.write_run(
        r#"{"run":"run_b","itinerary":"itn_1","agent":"analyst","outcome":"succeeded","started_at":"2026-09-16T10:00:00Z","finished_at":"2026-09-16T10:01:00Z","source":"unreported"}"#,
    );

    let (_, body) = call(factory.router(), "/runs?window=all_time").await;

    assert!(json(&body)["runs"][0]["cost_usd"].is_null());
    assert_eq!(json(&body)["runs"][0]["cost_source"], "unreported");
}

#[tokio::test]
async fn a_calendar_window_names_the_zone_it_was_reckoned_in_and_a_rolling_one_does_not() {
    // The absence is the point. Nobody should have to wonder which zone "last 7 days" used, and
    // everybody should be told which one "month to date" used.
    let factory = Factory::new("zone");

    let (_, calendar) = call(factory.router(), "/costs?window=month_to_date").await;
    assert_eq!(json(&calendar)["span"]["calendar"], true);
    assert!(json(&calendar)["span"]["zone"].is_string());

    let (_, rolling) = call(factory.router(), "/costs?window=last_7d").await;
    assert_eq!(json(&rolling)["span"]["calendar"], false);
    assert!(json(&rolling)["span"]["zone"].is_null());
}

#[tokio::test]
async fn one_unreported_run_downgrades_the_whole_total() {
    let factory = Factory::new("confidence");
    factory.write_run(
        r#"{"run":"run_c","itinerary":"itn_1","agent":"analyst","outcome":"succeeded","started_at":"2026-09-16T10:00:00Z","finished_at":"2026-09-16T10:01:00Z","usd":4.0,"source":"reported"}"#,
    );
    factory.write_run(
        r#"{"run":"run_d","itinerary":"itn_1","agent":"developer","outcome":"succeeded","started_at":"2026-09-16T10:00:00Z","finished_at":"2026-09-16T10:01:00Z","source":"unreported"}"#,
    );

    let (_, body) = call(factory.router(), "/costs?window=all_time").await;
    let total = &json(&body)["total"];

    assert_eq!(total["runs"], 2);
    assert_eq!(total["usd"], 4.0);
    assert_eq!(total["confidence"], "unreported");
    assert_eq!(total["measured_share"], 0.5);
}

#[tokio::test]
async fn a_run_still_going_is_listed_but_not_billed() {
    let factory = Factory::new("live");
    factory.write_run(
        r#"{"run":"run_e","itinerary":"itn_1","agent":"developer","outcome":"running","started_at":"2026-09-16T10:00:00Z","source":"unreported"}"#,
    );

    let (_, runs) = call(factory.router(), "/runs?window=all_time").await;
    assert_eq!(json(&runs)["runs"].as_array().map(Vec::len), Some(1));
    assert!(json(&runs)["runs"][0]["finished_at"].is_null());

    let (_, costs) = call(factory.router(), "/costs?window=all_time").await;
    assert_eq!(json(&costs)["total"]["runs"], 0);
}

#[tokio::test]
async fn control_operations_refuse_rather_than_pretend() {
    // A control that silently does nothing is worse than one that is not there, because it gets
    // trusted once and then relied upon.
    let factory = Factory::new("control");

    let response = factory
        .router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ground-stop")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn an_absent_run_is_a_404_with_the_identifier_in_it() {
    let factory = Factory::new("missing");

    let (status, body) = call(factory.router(), "/runs/run_nope").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(json(&body)["detail"].as_str().unwrap().contains("run_nope"));
}

#[tokio::test]
async fn an_empty_history_serves_zeroes_rather_than_failing() {
    // The dashboard has to work on a factory that has never run, which is every factory on the
    // day it is written.
    let factory = Factory::new("fresh");
    assert!(factory.path().join("history").is_dir());

    let (status, body) = call(factory.router(), "/costs?window=last_30d").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["total"]["runs"], 0);
    assert_eq!(json(&body)["total"]["usd"], 0.0);
    assert_eq!(
        json(&body)["total"]["confidence"],
        "reported",
        "nothing is unaccounted for when nothing has run"
    );
}

#[tokio::test]
async fn a_failed_run_colours_its_agent_on_the_route_map() {
    let factory = Factory::new("colour");
    factory.write_run(&format!(
        r#"{{"run":"run_f","itinerary":"itn_1","agent":"developer","outcome":"failed","started_at":"{now}","finished_at":"{now}","source":"unreported"}}"#,
        now = jiff::Timestamp::now()
    ));

    let (_, body) = call(factory.router(), "/graph").await;
    let svg = json(&body)["mermaid"].as_str().expect("svg").to_owned();

    assert!(
        svg.contains(r#"class="node agent failed" id="a_developer""#),
        "{svg}"
    );
}
