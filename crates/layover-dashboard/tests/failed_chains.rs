//! What the dashboard says about a chain that ended on a failed run, and a run the model service
//! ended: the chain is `failed` until a restart is queued, and the run says whose failure it was.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use layover_core::flight::{Flight, ItineraryId, Origin, RunId};
use layover_core::handover::{Interruption, Recovery};
use layover_core::queue::Queued;
use layover_dashboard::{Dashboard, DashboardState, Guard, router};
use layover_store::{History, Journal};
use tower::ServiceExt as _;

struct Factory(PathBuf);

impl Factory {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "layover-dash-failed-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("history")).expect("temp dirs");
        fs::write(root.join("layover.toml"), FACTORY).expect("writes config");
        Self(root)
    }

    /// Writes a run of `chain` that the model service ended a minute ago.
    fn provider_failure(&self, chain: &ItineraryId) {
        let at = jiff::Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_mins(1))
            .expect("in range");
        let day = at.to_string().chars().take(10).collect::<String>();
        let line = format!(
            r#"{{"run":"run_wolf","itinerary":"{chain}","agent":"wolf","outcome":"failed","started_at":"{at}","finished_at":"{at}","source":"copilot_credits","usd":0.3,"exit_code":1,"detail":"exited with code 1: the model service failed (http_400); not restarted: this agent's `recovery` policy is `manual`; a human decides","cause":"provider_error","request_id":"00000-7a12"}}"#,
            chain = chain.as_str()
        );
        fs::write(
            self.0.join("history").join(format!("runs-{day}.jsonl")),
            format!("{line}\n"),
        )
        .expect("writes history");
    }

    fn journal(&self) -> Journal {
        Journal::open(self.0.join("journal")).expect("opens journal")
    }

    async fn get(&self, path: &str) -> serde_json::Value {
        let app = router(
            Dashboard::new(DashboardState {
                config_path: self.0.join("layover.toml"),
                history: History::open(self.0.join("history")).expect("opens history"),
                journal: std::sync::Arc::new(self.journal()),
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
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        serde_json::from_slice(&bytes).expect("JSON")
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
cli = "copilot"

[agents.wolf]
prompt = "verify"
entry = true
"#;

#[tokio::test]
async fn a_run_the_model_service_ended_says_so_and_its_chain_needs_a_person() {
    let factory = Factory::new("dead");
    let chain = ItineraryId::generate();
    factory.provider_failure(&chain);

    let runs = factory.get("/runs?window=last_24h").await;
    let run = &runs["runs"][0];
    assert_eq!(run["status"], "failed", "never surfaced as succeeded");
    assert_eq!(run["cause"], "provider_error");
    assert_eq!(run["request_id"], "00000-7a12");

    let chains = factory.get("/itineraries?window=last_24h").await;
    assert_eq!(chains["failed"], 1, "{chains}");
    assert_eq!(chains["itineraries"][0]["state"], "failed");
    let detail = chains["itineraries"][0]["detail"]
        .as_str()
        .unwrap_or_default();
    assert!(
        detail.starts_with("`wolf` failed, and nothing came after it"),
        "{detail}"
    );

    let only_failed = factory
        .get("/itineraries?window=last_24h&state=failed")
        .await;
    assert_eq!(only_failed["itineraries"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn a_chain_whose_restart_is_backing_off_is_working_and_the_queue_says_when_it_starts() {
    let factory = Factory::new("restarting");
    let chain = ItineraryId::generate();
    factory.provider_failure(&chain);
    let at: jiff::Timestamp = jiff::Timestamp::now() + jiff::SignedDuration::from_mins(1);
    factory
        .journal()
        .queue(
            Queued::new(
                Flight::new(chain.clone(), Origin::Human, "wolf".into(), "verify", 6),
                None,
                BTreeMap::new(),
            )
            .restarting(Recovery {
                previous: RunId::from("run_wolf"),
                interruption: Interruption::ProviderError {
                    request_id: Some("00000-7a12".to_owned()),
                },
                attempt: 2,
            })
            .waiting_until(at),
        )
        .expect("queues");

    let chains = factory.get("/itineraries?window=last_24h").await;
    assert_eq!(chains["failed"], 0, "{chains}");
    assert_eq!(chains["itineraries"][0]["state"], "working");
    assert_eq!(
        chains["itineraries"][0]["detail"],
        "`wolf` restarts after the model service failed — attempt 2, once its wait is over"
    );

    let flights = factory.get("/flights").await;
    let restart = &flights["pending"][0];
    assert_eq!(restart["attempt"], 2);
    assert_eq!(restart["not_before"], at.to_string());
}
