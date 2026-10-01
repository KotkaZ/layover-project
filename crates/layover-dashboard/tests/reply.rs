//! Answering a help request from the dashboard continues the work, as the chain that asked had it.
//!
//! `devforge`'s case: the agent wrote a spec, found decisions only a person could make, filed a fatal
//! request and ended. The chain looked finished, "Resolved" restarted nothing, and the only way on
//! was a trigger that started from the workflow's defaults.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use jiff::Timestamp;
use layover_core::agent::AgentName;
use layover_core::flight::{ItineraryId, Origin, RunId};
use layover_core::help::{Blocker, HelpRequest};
use layover_core::pipeline::PipelineName;
use layover_core::run::{Outcome, RunRecord};
use layover_core::scope::ChainScope;
use layover_dashboard::{Dashboard, DashboardState, Guard, router};
use layover_store::{History, Journal};
use tower::ServiceExt as _;

struct Factory(PathBuf);

impl Factory {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("layover-dash-reply-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".layover").join("history")).expect("dirs");
        fs::write(
            root.join("layover.toml"),
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "copilot"
max_hops = 9

[runners.copilot]
command = ["copilot"]

[agents.analyst]
prompt = "specify"

[agents.bob]
prompt = "build"

[pipelines.devforge]
entry = "analyst"

[pipelines.devforge.flags]
publish_pr = { default = false, description = "Open the pull request when done." }
run_e2e = { default = false, description = "Run the end-to-end suite." }

[[routes]]
from = "analyst"
to = "bob"
"#,
        )
        .expect("config");
        Self(root)
    }

    fn journal(&self) -> Journal {
        Journal::open(self.0.join(".layover").join("journal")).expect("journal")
    }

    fn history(&self) -> History {
        History::open(self.0.join(".layover").join("history")).expect("history")
    }

    fn app(&self) -> Router {
        let layover = self.0.join(".layover");
        router(
            Dashboard::new(DashboardState {
                config_path: self.0.join("layover.toml"),
                history: self.history(),
                journal: std::sync::Arc::new(self.journal()),
                ground_stop: layover.join("ground-stop"),
            }),
            Guard::Open,
        )
    }

    /// The chain that asked: one finished run of `bob` in `devforge`, allowed to publish, which filed
    /// a fatal request. `recorded` says whether the request records its chain, as one filed from
    /// now on does.
    fn asked(&self, recorded: bool) -> HelpRequest {
        let chain = ItineraryId::generate();
        let run = RunId::generate();
        let flags = BTreeMap::from([
            ("publish_pr".to_owned(), true),
            ("run_e2e".to_owned(), false),
        ]);

        let mut record = RunRecord::started(
            run.clone(),
            chain.clone(),
            AgentName::new("bob"),
            Timestamp::now(),
        );
        record.outcome = Outcome::Succeeded;
        record.finished_at = Some(Timestamp::now());
        record.pipeline = Some(PipelineName::new("devforge"));
        record.flags = if recorded {
            flags.clone()
        } else {
            BTreeMap::new()
        };
        self.history().append(&record).expect("records");

        let mut request = HelpRequest::new(
            AgentName::new("bob"),
            run,
            chain,
            Blocker::Decision,
            "spec needs 3 decisions from Karl",
            "1. Which retry policy?\n2. Who owns the alert?\n3. Ship behind a flag?",
            Timestamp::now(),
        )
        .fatal();
        if recorded {
            request = request.raised_in(
                ChainScope::new(
                    Some(PipelineName::new("devforge")),
                    BTreeSet::from([Some(PipelineName::new("follow-up"))]),
                ),
                flags,
            );
        }
        self.journal().ask(&request).expect("asks");
        request
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

async fn call(
    app: Router,
    method: &str,
    path: &str,
    body: &serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
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

async fn get(app: Router, path: &str) -> serde_json::Value {
    let response = app
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("responds");
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json")
}

#[tokio::test]
async fn a_reply_continues_the_work_as_the_chain_that_asked_had_it() {
    let factory = Factory::new("continues");
    let asked = factory.asked(true);

    let (status, replied) = call(
        factory.app(),
        "POST",
        "/help/reply",
        &serde_json::json!({
            "run_id": asked.run.as_str(),
            "body": "1. Exponential.\n2. The platform team.\n3. Yes.",
            "by": "Karl",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{replied}");

    let pending = factory.journal().pending().expect("queue");
    assert_eq!(pending.len(), 1, "exactly one flight");
    let queued = &pending[0];
    assert_eq!(
        queued.flight.to.as_str(),
        "bob",
        "to the agent that asked, not the entry"
    );
    assert_eq!(queued.flight.from, Origin::Human, "a person sent it");
    assert_ne!(queued.flight.itinerary, asked.itinerary, "a new chain");
    assert_eq!(queued.flight.hops_remaining, 9, "with a fresh budget");
    assert_eq!(queued.pipeline, Some(PipelineName::new("devforge")));
    assert_eq!(
        queued.flags,
        BTreeMap::from([
            ("publish_pr".to_owned(), true),
            ("run_e2e".to_owned(), false)
        ]),
        "the chain's flags, not the defaults, which would forbid publishing"
    );
    assert_eq!(
        queued.within,
        BTreeSet::from([Some(PipelineName::new("follow-up"))]),
        "the chain's reach, not a wider one"
    );
    assert_eq!(
        queued.continues.as_ref(),
        Some(&asked.itinerary),
        "linked to the chain that asked"
    );
    assert_eq!(
        queued.flight.body,
        format!(
            "In reply to your help request {} (spec needs 3 decisions from Karl)\n\n1. Exponential.\n2. The platform team.\n3. Yes.",
            asked.run
        )
    );
    assert_eq!(replied["itinerary_id"], queued.flight.itinerary.as_str());
    assert_eq!(replied["answered"], 1);

    let help = get(factory.app(), "/help?window=last_24h").await;
    let request = &help["requests"][0];
    assert!(!request["resolved_at"].is_null(), "answered is dealt with");
    assert_eq!(request["reply"]["by"], "Karl");
    assert_eq!(
        request["reply"]["itinerary_id"],
        queued.flight.itinerary.as_str()
    );
    assert_eq!(help["open"], 0);
}

#[tokio::test]
async fn a_request_already_dealt_with_or_unknown_is_not_answered_twice() {
    let factory = Factory::new("twice");
    let asked = factory.asked(true);
    let reply = serde_json::json!({ "run_id": asked.run.as_str(), "body": "Go." });

    let (first, _) = call(factory.app(), "POST", "/help/reply", &reply).await;
    let (second, problem) = call(factory.app(), "POST", "/help/reply", &reply).await;
    assert_eq!(first, StatusCode::ACCEPTED);
    assert_eq!(second, StatusCode::CONFLICT, "{problem}");
    assert_eq!(
        factory.journal().pending().expect("queue").len(),
        1,
        "one chain, not two"
    );

    let resolved = factory.asked(true);
    let (status, _) = call(
        factory.app(),
        "POST",
        "/help/resolve",
        &serde_json::json!({ "run_id": resolved.run.as_str() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        factory.app(),
        "POST",
        "/help/reply",
        &serde_json::json!({ "run_id": resolved.run.as_str(), "body": "Go." }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "resolved is dealt with");

    let (status, _) = call(
        factory.app(),
        "POST",
        "/help/reply",
        &serde_json::json!({ "run_id": "run_01NOSUCHRUN", "body": "Go." }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = call(
        factory.app(),
        "POST",
        "/help/reply",
        &serde_json::json!({ "run_id": factory.asked(true).run.as_str(), "body": "  " }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "nothing to say");
}

#[tokio::test]
async fn a_request_that_predates_recorded_flags_needs_them_said_and_never_takes_the_defaults() {
    let factory = Factory::new("legacy");
    let asked = factory.asked(false);
    let help = get(factory.app(), "/help?window=last_24h").await;
    assert!(
        help["requests"][0]["flags"].is_null(),
        "the page knows to ask"
    );
    assert_eq!(
        help["requests"][0]["pipeline"], "devforge",
        "placed by its chain's runs"
    );

    let (status, problem) = call(
        factory.app(),
        "POST",
        "/help/reply",
        &serde_json::json!({ "run_id": asked.run.as_str(), "body": "Go." }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    assert!(
        problem["detail"]
            .as_str()
            .is_some_and(|d| d.contains("`publish_pr`")),
        "{problem}"
    );
    assert!(factory.journal().pending().expect("queue").is_empty());

    let (status, replied) = call(
        factory.app(),
        "POST",
        "/help/reply",
        &serde_json::json!({ "run_id": asked.run.as_str(), "body": "Go.", "flags": { "publish_pr": true } }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{replied}");
    let queued = &factory.journal().pending().expect("queue")[0];
    assert_eq!(queued.pipeline, Some(PipelineName::new("devforge")));
    assert_eq!(queued.flags.get("publish_pr"), Some(&true));
    assert_eq!(
        queued.flags.get("run_e2e"),
        Some(&false),
        "an unsaid flag takes its default"
    );
}

#[tokio::test]
async fn flags_are_not_chosen_for_a_request_that_records_them_and_a_ground_stop_refuses_a_reply() {
    let factory = Factory::new("refused");
    let asked = factory.asked(true);

    let (status, _) = call(
        factory.app(),
        "POST",
        "/help/reply",
        &serde_json::json!({ "run_id": asked.run.as_str(), "body": "Go.", "flags": { "publish_pr": false } }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the recorded flags are what a reply carries"
    );

    fs::write(factory.0.join(".layover").join("ground-stop"), "").expect("engages");
    let (status, _) = call(
        factory.app(),
        "POST",
        "/help/reply",
        &serde_json::json!({ "run_id": asked.run.as_str(), "body": "Go." }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(factory.journal().pending().expect("queue").is_empty());
}

#[tokio::test]
async fn a_chain_waiting_for_an_answer_says_so_and_is_finished_once_dealt_with() {
    let factory = Factory::new("waiting");
    let asked = factory.asked(true);

    let chains = get(factory.app(), "/itineraries?window=last_24h").await;
    let chain = &chains["itineraries"][0];
    assert_eq!(chain["state"], "awaiting_human", "{chains}");
    assert_eq!(chain["waiting_for"]["run_id"], asked.run.as_str());
    assert_eq!(
        chain["waiting_for"]["summary"],
        "spec needs 3 decisions from Karl"
    );
    assert_eq!(
        chain["flags"]["publish_pr"], true,
        "what Continue… starts from"
    );
    assert_eq!(chains["awaiting_human"], 1);
    let filtered = get(
        factory.app(),
        "/itineraries?window=last_24h&state=awaiting_human",
    )
    .await;
    assert_eq!(filtered["itineraries"].as_array().map(Vec::len), Some(1));

    // Resolved without a reply: somebody decided nothing more is needed.
    let (status, _) = call(
        factory.app(),
        "POST",
        "/help/resolve",
        &serde_json::json!({ "run_id": asked.run.as_str() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let chains = get(factory.app(), "/itineraries?window=last_24h").await;
    assert_eq!(chains["itineraries"][0]["state"], "finished");
    assert_eq!(chains["awaiting_human"], 0);

    // Answered: finished, and it says where the work went.
    let answered = factory.asked(true);
    let (_, replied) = call(
        factory.app(),
        "POST",
        "/help/reply",
        &serde_json::json!({ "run_id": answered.run.as_str(), "body": "Go." }),
    )
    .await;
    let chains = get(factory.app(), "/itineraries?window=last_24h").await;
    let chain = chains["itineraries"]
        .as_array()
        .and_then(|all| {
            all.iter()
                .find(|chain| chain["itinerary_id"] == answered.itinerary.as_str())
        })
        .expect("listed")
        .clone();
    assert_eq!(chain["state"], "finished");
    assert_eq!(chain["continued_by"][0], replied["itinerary_id"]);
}
