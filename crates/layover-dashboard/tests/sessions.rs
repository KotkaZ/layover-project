//! Watching runs from the dashboard: what is running now, and what each run's CLI is printing.
//!
//! Against a real factory directory, with live records and transcripts written where the Tower
//! writes them, because the point is that the dashboard needs nothing from the Tower but its files.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use jiff::Timestamp;
use layover_core::agent::AgentName;
use layover_core::flight::{Flight, ItineraryId, Origin, RunId};
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_core::run::{Outcome, RunRecord};
use layover_dashboard::{Dashboard, DashboardState, Guard, router};
use layover_store::live::{Ledger, Live};
use layover_store::{History, Journal};
use tower::ServiceExt as _;

struct Factory(PathBuf);

impl Factory {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "layover-dash-sessions-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".layover").join("history")).expect("dirs");
        fs::write(
            root.join("layover.toml"),
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "copilot"

[runners.copilot]
command = ["copilot", "--model", "claude-opus-5.5"]

[agents.eagle]
prompt = "review"
entry = true

[pipelines.devforge]
entry = "eagle"
"#,
        )
        .expect("config");
        Self(root)
    }

    fn app(&self) -> Router {
        let layover = self.0.join(".layover");
        router(
            Dashboard::new(DashboardState {
                config_path: self.0.join("layover.toml"),
                history: History::open(layover.join("history")).expect("history"),
                journal: std::sync::Arc::new(
                    Journal::open(layover.join("journal")).expect("journal"),
                ),
                ground_stop: layover.join("ground-stop"),
            }),
            Guard::Open,
        )
    }

    fn ledger(&self) -> Ledger {
        Ledger::open(self.0.join(".layover").join("state").join("runs")).expect("ledger")
    }

    fn transcript(&self, run: &RunId) -> PathBuf {
        let dir = layover_store::hangar::run_dir(
            &self.0.join(".layover").join("hangars"),
            &AgentName::new("eagle"),
            run,
        );
        fs::create_dir_all(&dir).expect("hangar");
        dir.join(layover_store::hangar::TRANSCRIPT)
    }

    /// A run of `eagle` in the `devforge` workflow, alive as far as the Tower's records go.
    fn running(&self) -> Live {
        let run = RunId::generate();
        let live = Live {
            hangar: self
                .transcript(&run)
                .parent()
                .expect("hangar")
                .to_path_buf(),
            run,
            itinerary: ItineraryId::generate(),
            agent: AgentName::new("eagle"),
            pid: 4242,
            started_at: Timestamp::now(),
            queued: Some(Queued::new(
                Flight::new(
                    ItineraryId::generate(),
                    Origin::Human,
                    AgentName::new("eagle"),
                    "go",
                    4,
                ),
                Some(PipelineName::new("devforge")),
                BTreeMap::new(),
            )),
            owner: None,
            sent_by: None,
        };
        self.ledger().starting(&live).expect("records");
        live
    }

    /// Ends `live` the way the Tower does: history first, then the live record.
    fn finish(&self, live: &Live) {
        let mut record = RunRecord::started(
            live.run.clone(),
            live.itinerary.clone(),
            live.agent.clone(),
            live.started_at,
        );
        record.outcome = Outcome::Succeeded;
        record.finished_at = Some(Timestamp::now());
        History::open(self.0.join(".layover").join("history"))
            .expect("history")
            .append(&record)
            .expect("appends");
        self.ledger().finished(&live.run).expect("forgets");
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn event(kind: &str, data: &str) -> String {
    format!("{{\"type\":\"{kind}\",\"timestamp\":\"2026-10-01T08:00:00.000Z\",\"data\":{data}}}\n")
}

fn append(path: &Path, text: &str) {
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("opens")
        .write_all(text.as_bytes())
        .expect("writes");
}

const VIEW: &str = r#"{"toolCallId":"c1","toolName":"view","arguments":{"path":"src/lib.rs"}}"#;
const SAID: &str = r#"{"messageId":"m1","content":"The change is sound."}"#;

async fn json(app: Router, path: &str) -> (StatusCode, serde_json::Value) {
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
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

async fn open(app: Router, path: &str) -> (StatusCode, Body) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("responds");
    (response.status(), response.into_body())
}

/// Reads the stream until `seen` is in what has arrived, failing if it does not come.
async fn until(body: &mut Body, so_far: &mut String, seen: &str) {
    while !so_far.contains(seen) {
        let frame = tokio::time::timeout(Duration::from_secs(10), body.frame())
            .await
            .unwrap_or_else(|_| panic!("waited for {seen:?}; had {so_far}"))
            .unwrap_or_else(|| panic!("the stream ended before {seen:?}; had {so_far}"))
            .expect("a frame");
        if let Ok(data) = frame.into_data() {
            so_far.push_str(&String::from_utf8_lossy(&data));
        }
    }
}

#[tokio::test]
async fn a_running_session_is_listed_as_running_with_its_workflow_and_model() {
    // History holds a run only once it is over, so a dashboard that read only history never showed
    // anything running.
    let factory = Factory::new("listed");
    let live = factory.running();

    let (status, listed) = json(factory.app(), "/runs").await;
    assert_eq!(status, StatusCode::OK);
    let run = &listed["runs"][0];
    assert_eq!(run["run_id"], live.run.as_str());
    assert_eq!(run["status"], "running");
    assert_eq!(run["pipeline"], "devforge");
    assert_eq!(run["model"], "claude-opus-5.5");
    assert!(run["cost_usd"].is_null(), "nothing is billed until it ends");

    let (_, one) = json(factory.app(), &format!("/runs/{}", live.run)).await;
    assert_eq!(one["status"], "running");

    let (_, finished) = json(factory.app(), "/runs?status=succeeded").await;
    assert_eq!(finished["runs"].as_array().map(Vec::len), Some(0));

    let (_, map) = json(factory.app(), "/graph").await;
    let svg = map["mermaid"].as_str().expect("svg");
    assert!(
        svg.contains(r#"class="node agent running" id="a_eagle""#),
        "{svg}"
    );
}

#[tokio::test]
async fn a_chain_whose_only_run_is_alive_is_listed_as_working() {
    // A chain was built from history alone, so one whose first run had not ended yet — a trigger a
    // minute old, or the chain a reply just started — was not on the Chains page at all.
    let factory = Factory::new("chain-live");
    let live = factory.running();

    let (_, chains) = json(factory.app(), "/itineraries?window=last_24h").await;
    let chain = &chains["itineraries"][0];
    assert_eq!(chain["itinerary_id"], live.itinerary.as_str(), "{chains}");
    assert_eq!(chain["state"], "working");
    assert_eq!(chain["pipeline"], "devforge");
}

#[tokio::test]
async fn a_finished_run_replays_from_its_transcript_and_says_how_it_ended() {
    let factory = Factory::new("replay");
    let live = factory.running();
    let transcript = factory.transcript(&live.run);
    append(
        &transcript,
        &event(
            "assistant.reasoning_delta",
            r#"{"reasoningId":"r1","deltaContent":"Look"}"#,
        ),
    );
    append(
        &transcript,
        &event(
            "assistant.reasoning",
            r#"{"reasoningId":"r1","content":"Look at the diff."}"#,
        ),
    );
    append(&transcript, &event("tool.execution_start", VIEW));
    append(&transcript, &event("assistant.message", SAID));
    factory.finish(&live);

    let (status, body) = open(factory.app(), &format!("/runs/{}/stream", live.run)).await;
    assert_eq!(status, StatusCode::OK);
    let text =
        String::from_utf8_lossy(&body.collect().await.expect("ends").to_bytes()).into_owned();

    assert!(
        text.contains(r#""kind":"think","text":"Look at the diff.""#),
        "{text}"
    );
    assert!(
        text.contains(r#""kind":"tool","text":"view src/lib.rs""#),
        "{text}"
    );
    assert!(
        text.contains(r#""kind":"say","text":"The change is sound.""#),
        "{text}"
    );
    assert!(
        !text.contains("event: partial"),
        "a replay has nothing still arriving: {text}"
    );
    assert!(
        text.trim_end()
            .ends_with(r#"{"detail":null,"status":"succeeded"}"#),
        "{text}"
    );
}

#[tokio::test]
async fn a_long_replay_shows_nothing_as_still_arriving_however_it_is_read() {
    // A real forty-minute transcript is tens of megabytes and read a chunk at a time; a block whose
    // deltas straddle two chunks is not still being written, and must not flash past as typing.
    let factory = Factory::new("long-replay");
    let live = factory.running();
    let transcript = factory.transcript(&live.run);
    let word = "x".repeat(300);
    let mut text = String::new();
    for n in 0..4_000 {
        text.push_str(&event(
            "assistant.reasoning_delta",
            &format!(
                r#"{{"reasoningId":"r{}","deltaContent":"{word}"}}"#,
                n / 1_000
            ),
        ));
        if n % 1_000 == 999 {
            text.push_str(&event(
                "assistant.reasoning",
                &format!(
                    r#"{{"reasoningId":"r{}","content":"block {}"}}"#,
                    n / 1_000,
                    n / 1_000
                ),
            ));
        }
    }
    append(&transcript, &text);
    assert!(text.len() > 1 << 20, "longer than one chunk");
    factory.finish(&live);

    let (_, body) = open(factory.app(), &format!("/runs/{}/stream", live.run)).await;
    let text =
        String::from_utf8_lossy(&body.collect().await.expect("ends").to_bytes()).into_owned();

    assert!(text.contains("block 3"), "{}", &text[..text.len().min(400)]);
    assert!(
        !text.contains("event: partial"),
        "a finished run has nothing still arriving"
    );
}

#[tokio::test]
async fn a_live_run_is_followed_as_it_writes_and_the_stream_ends_with_the_run() {
    let factory = Factory::new("follow");
    let live = factory.running();
    let transcript = factory.transcript(&live.run);
    append(&transcript, &event("tool.execution_start", VIEW));

    let (status, mut body) = open(factory.app(), &format!("/runs/{}/stream", live.run)).await;
    assert_eq!(status, StatusCode::OK);
    let mut seen = String::new();
    until(&mut body, &mut seen, "view src/lib.rs").await;

    // The model is typing: shown as it arrives, then replaced by the finished message.
    append(
        &transcript,
        &event(
            "assistant.message_delta",
            r#"{"messageId":"m1","deltaContent":"The change"}"#,
        ),
    );
    until(&mut body, &mut seen, r#""text":"The change"}"#).await;
    assert!(seen.contains("event: partial"), "{seen}");
    // Half a line is not read until the CLI finishes writing it.
    append(&transcript, &event("assistant.message", SAID)[..30]);
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    append(&transcript, &event("assistant.message", SAID)[30..]);
    until(&mut body, &mut seen, "The change is sound.").await;
    assert!(
        seen.contains(r#""closes":"m:m1""#),
        "the message replaces what was typed: {seen}"
    );

    factory.finish(&live);
    until(&mut body, &mut seen, "event: end").await;
    assert!(seen.contains(r#""status":"succeeded""#), "{seen}");
    let after = tokio::time::timeout(Duration::from_secs(5), body.frame())
        .await
        .expect("closes");
    assert!(after.is_none(), "the stream ends with the run");
}

#[tokio::test]
async fn a_viewer_that_reconnects_is_sent_only_what_it_had_not_seen() {
    let factory = Factory::new("resume");
    let live = factory.running();
    let transcript = factory.transcript(&live.run);
    let first = event("tool.execution_start", VIEW);
    append(&transcript, &first);
    append(&transcript, &event("assistant.message", SAID));
    factory.finish(&live);

    let (_, body) = open(
        factory.app(),
        &format!("/runs/{}/stream?after={}", live.run, first.len()),
    )
    .await;
    let text =
        String::from_utf8_lossy(&body.collect().await.expect("ends").to_bytes()).into_owned();
    assert!(!text.contains("view src/lib.rs"), "{text}");
    assert!(text.contains("The change is sound."), "{text}");

    // An offset in the middle of a line starts at the next whole one, not half-way through it.
    let (_, body) = open(
        factory.app(),
        &format!("/runs/{}/stream?after=10", live.run),
    )
    .await;
    let text =
        String::from_utf8_lossy(&body.collect().await.expect("ends").to_bytes()).into_owned();
    assert!(
        !text.contains(r#""kind":"raw""#),
        "half a line is not shown as output: {text}"
    );
    assert!(!text.contains("view src/lib.rs"), "{text}");
    assert!(text.contains("The change is sound."), "{text}");
}

#[tokio::test]
async fn a_run_with_no_transcript_says_so_and_an_unknown_one_is_not_found() {
    let factory = Factory::new("missing");
    let live = factory.running();
    let _ = fs::remove_dir_all(factory.transcript(&live.run).parent().expect("hangar"));
    factory.finish(&live);

    let (status, body) = open(factory.app(), &format!("/runs/{}/stream", live.run)).await;
    assert_eq!(status, StatusCode::OK);
    let text =
        String::from_utf8_lossy(&body.collect().await.expect("ends").to_bytes()).into_owned();
    assert!(text.contains(r#""missing":true"#), "{text}");

    for unknown in ["run_01NOSUCHRUN", "..%2F..%2Flayover.toml", "a%20b"] {
        let (status, _) = open(factory.app(), &format!("/runs/{unknown}/stream")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{unknown}");
    }
}
