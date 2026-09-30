//! What the dashboard says about Copilot runs priced from their credits.

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
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "layover-dash-credits-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("history")).expect("temp dirs");
        fs::write(root.join("layover.toml"), FACTORY).expect("writes config");
        Self(root)
    }

    /// Writes a finished run an hour ago, costing `usd` from `source`.
    fn run(&self, run: &str, usd: f64, source: &str) {
        let at = jiff::Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_hours(1))
            .expect("in range");
        let day = at.to_string().chars().take(10).collect::<String>();
        let path = self.0.join("history").join(format!("runs-{day}.jsonl"));
        let line = format!(
            r#"{{"run":"{run}","itinerary":"itn_{run}","agent":"reviewer","outcome":"succeeded","started_at":"{at}","finished_at":"{at}","usd":{usd},"source":"{source}"}}"#
        );
        let mut existing = fs::read_to_string(&path).unwrap_or_default();
        existing.push_str(&line);
        existing.push('\n');
        fs::write(path, existing).expect("writes history");
    }

    async fn get(&self, path: &str) -> (StatusCode, String) {
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
runner = "copilot"

[runners.copilot]
command = ["copilot", "--output-format", "json"]

[reserve]
fuel_usd = 20.0
window_hours = 24

[agents.reviewer]
prompt = "review"
entry = true
"#;

fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).expect("JSON")
}

#[tokio::test]
async fn credit_priced_runs_are_measured_and_named_in_the_totals() {
    let factory = Factory::new("measured");
    factory.run("run_a", 15.11, "copilot_credits");
    factory.run("run_b", 8.20, "copilot_credits");

    let (status, body) = factory.get("/costs?window=last_24h").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let total = &json(&body)["total"];

    assert_eq!(total["credit_runs"], 2, "{total}");
    assert_eq!(total["unreported_runs"], 0, "{total}");
    assert_eq!(total["confidence"], "copilot_credits", "{total}");
    assert_eq!(total["measured_share"], 1.0, "{total}");
    assert!((total["usd"].as_f64().expect("usd") - 23.31).abs() < 1e-9);
}

#[tokio::test]
async fn a_copilot_run_that_reported_nothing_still_makes_the_total_a_lower_bound() {
    let factory = Factory::new("hole");
    factory.run("run_a", 15.11, "copilot_credits");
    factory.run("run_b", 0.0, "unreported");

    let (_, body) = factory.get("/costs?window=last_24h").await;
    let total = &json(&body)["total"];

    assert_eq!(total["confidence"], "unreported", "{total}");
    assert_eq!(total["measured_share"], 0.5, "{total}");
}

#[tokio::test]
async fn credit_priced_spend_draws_on_the_reserve_the_dashboard_shows() {
    let factory = Factory::new("reserve");
    factory.run("run_a", 15.11, "copilot_credits");
    factory.run("run_b", 8.20, "copilot_credits");

    let (_, body) = factory.get("/costs?window=last_24h").await;
    let reserve = &json(&body)["reserve"];

    assert!((reserve["spent_usd"].as_f64().expect("spent") - 23.31).abs() < 1e-9);
    assert_eq!(reserve["exhausted"], true, "{reserve}");
}

#[tokio::test]
async fn a_run_says_it_was_priced_from_copilot_credits() {
    let factory = Factory::new("run");
    factory.run("run_a", 15.11, "copilot_credits");

    let (_, body) = factory.get("/runs?window=all_time").await;
    assert_eq!(
        json(&body)["runs"][0]["cost_source"],
        "copilot_credits",
        "{body}"
    );
}

#[tokio::test]
async fn the_page_names_credit_pricing_in_its_provenance_line() {
    let factory = Factory::new("page");
    let (_, script) = factory.get("/app.js").await;

    assert!(script.contains("priced from Copilot credits"), "{script}");
}
