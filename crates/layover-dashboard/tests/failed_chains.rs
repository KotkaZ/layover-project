//! What the dashboard says about a chain that ended on a failed run, and a run the model service
//! ended: the chain is `failed` until a restart is queued, the run says whose failure it was, and a
//! person can dismiss the chain once they have dealt with it.

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
        self.ran(
            chain,
            r#""outcome":"failed","source":"copilot_credits","usd":0.3,"exit_code":1,"detail":"exited with code 1: the model service failed (http_400); not restarted: this agent's `recovery` policy is `manual`; a human decides","cause":"provider_error","request_id":"00000-7a12""#,
        );
    }

    /// Writes a run of `chain` that went well, a minute ago.
    fn success(&self, chain: &ItineraryId) {
        self.ran(chain, r#""outcome":"succeeded","source":"unreported""#);
    }

    fn ran(&self, chain: &ItineraryId, how: &str) {
        let at = jiff::Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_mins(1))
            .expect("in range");
        let day = at.to_string().chars().take(10).collect::<String>();
        let line = format!(
            r#"{{"run":"run_wolf","itinerary":"{chain}","agent":"wolf","started_at":"{at}","finished_at":"{at}",{how}}}"#,
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
        let (status, body) = self.send("GET", path, None).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        body
    }

    async fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let app = router(
            Dashboard::new(DashboardState {
                config_path: self.0.join("layover.toml"),
                history: History::open(self.0.join("history")).expect("opens history"),
                journal: std::sync::Arc::new(self.journal()),
                ground_stop: self.0.join("ground-stop"),
            }),
            Guard::Open,
        );
        let request = Request::builder().method(method).uri(path);
        let request = match body {
            Some(body) => request
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned())),
            None => request.body(Body::empty()),
        };
        let response = app
            .oneshot(request.expect("request"))
            .await
            .expect("responds");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        (status, serde_json::from_slice(&bytes).expect("JSON"))
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

#[tokio::test]
async fn a_failed_chain_somebody_dismissed_stays_failed_and_stops_needing_anybody() {
    let factory = Factory::new("dismissed");
    let chain = ItineraryId::generate();
    factory.provider_failure(&chain);
    // It starts nothing, so a Ground Stop does not refuse it.
    fs::write(factory.0.join("ground-stop"), "").expect("engages");
    let path = format!("/itineraries/{}/dismiss", chain.as_str());

    let (status, dismissed) = factory
        .send(
            "POST",
            &path,
            Some(r#"{"by":"karl","note":"  merged the fix by hand  "}"#),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{dismissed}");
    assert_eq!(
        dismissed["state"], "failed",
        "what happened stays on record"
    );
    assert_eq!(dismissed["dismissed"]["by"], "karl");
    assert_eq!(dismissed["dismissed"]["note"], "merged the fix by hand");
    assert_eq!(dismissed["dismissed"]["run_id"], "run_wolf");

    let chains = factory.get("/itineraries?window=last_24h").await;
    assert_eq!(chains["failed"], 1, "{chains}");
    assert_eq!(chains["dismissed"], 1, "{chains}");
    assert_eq!(
        chains["itineraries"][0]["dismissed"],
        dismissed["dismissed"]
    );

    // A second press, by somebody else, keeps who did it first.
    let (status, again) = factory.send("POST", &path, Some(r#"{"by":"ada"}"#)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["dismissed"], dismissed["dismissed"]);

    let (status, undone) = factory.send("DELETE", &path, None).await;
    assert_eq!(status, StatusCode::OK, "{undone}");
    assert!(undone["dismissed"].is_null(), "{undone}");
    let chains = factory.get("/itineraries?window=last_24h").await;
    assert_eq!(chains["dismissed"], 0, "it needs attention again: {chains}");

    let (status, _) = factory.send("DELETE", &path, None).await;
    assert_eq!(status, StatusCode::OK, "undoing twice changes nothing");
}

#[tokio::test]
async fn only_a_stalled_or_failed_chain_can_be_dismissed() {
    let factory = Factory::new("refused");
    let chain = ItineraryId::generate();
    factory.success(&chain);

    let (status, refused) = factory
        .send(
            "POST",
            &format!("/itineraries/{}/dismiss", chain.as_str()),
            Some("{}"),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["title"], "it finished");

    let unknown = format!("/itineraries/{}/dismiss", ItineraryId::generate().as_str());
    let (status, _) = factory.send("POST", &unknown, Some("{}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = factory.send("DELETE", &unknown, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
