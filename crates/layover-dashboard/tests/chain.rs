//! One chain, whole: what it ran, where its work went, and what it waits for — drawn on its own
//! even while other chains of the same workflow run beside it.
//!
//! The case this exists for is a development workflow triggered several times at once. Its route
//! map colours an agent while any of those chains runs it, so the map alone cannot say which chain
//! is where.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use jiff::{SignedDuration, Timestamp};
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
        let root =
            std::env::temp_dir().join(format!("layover-dash-chain-{label}-{}", std::process::id()));
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
command = ["copilot"]

[agents.analyst]
prompt = "analyse"

[agents.coder]
prompt = "code"

[agents.reviewer]
prompt = "review"

[agents.helper]
prompt = "help"

[pipelines.devforge]
entry = "analyst"

[pipelines.nightly]
entry = "coder"

[[routes]]
from = "analyst"
to = ["coder", "helper"]

[[routes]]
from = "coder"
to = "reviewer"

[[routes]]
from = "reviewer"
to = "coder"
"#,
        )
        .expect("config");
        Self(root)
    }

    fn layover(&self) -> PathBuf {
        self.0.join(".layover")
    }

    fn app(&self) -> Router {
        router(
            Dashboard::new(DashboardState {
                config_path: self.0.join("layover.toml"),
                history: History::open(self.layover().join("history")).expect("history"),
                journal: std::sync::Arc::new(
                    Journal::open(self.layover().join("journal")).expect("journal"),
                ),
                ground_stop: self.layover().join("ground-stop"),
            }),
            Guard::Open,
        )
    }

    /// A finished run, `minutes` ago, sent by `sent_by`.
    fn ran(
        &self,
        chain: &ItineraryId,
        agent: &str,
        minutes: i64,
        outcome: Outcome,
        sent_by: Option<&[&str]>,
    ) {
        let at = Timestamp::now() - SignedDuration::from_mins(minutes);
        let mut record = RunRecord::started(RunId::generate(), chain.clone(), agent.into(), at)
            .from_pipeline(PipelineName::new("devforge"))
            .finished(outcome, at + SignedDuration::from_secs(30));
        record.sent_by = sent_by.map(|names| names.iter().map(|&name| name.into()).collect());
        History::open(self.layover().join("history"))
            .expect("history")
            .append(&record)
            .expect("appends");
    }

    /// A run alive now, in `pipeline`, sent by `from`.
    fn running(&self, chain: &ItineraryId, agent: &str, pipeline: &str, from: &str) {
        let flight = Flight::new(
            chain.clone(),
            Origin::Agent(from.into()),
            agent.into(),
            "go",
            4,
        );
        let live = Live {
            run: RunId::generate(),
            itinerary: chain.clone(),
            agent: agent.into(),
            pid: 4242,
            started_at: Timestamp::now(),
            hangar: self.0.join("hangar"),
            queued: Some(Queued::new(
                flight,
                Some(PipelineName::new(pipeline)),
                BTreeMap::new(),
            )),
            owner: None,
            sent_by: Some(vec![AgentName::new(from)]),
        };
        Ledger::open(self.layover().join("state").join("runs"))
            .expect("ledger")
            .starting(&live)
            .expect("records");
    }

    /// A trigger of `devforge` waiting for a slot.
    fn queued(&self, chain: &ItineraryId) {
        let flight = Flight::new(chain.clone(), Origin::Human, "analyst".into(), "go", 4);
        Journal::open(self.layover().join("journal"))
            .expect("journal")
            .queue(Queued::new(
                flight,
                Some(PipelineName::new("devforge")),
                BTreeMap::new(),
            ))
            .expect("queues");
    }
}

impl Drop for Factory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

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

/// The opening tag of an agent's box.
fn node<'a>(svg: &'a str, agent: &str) -> &'a str {
    let at = svg
        .find(&format!(r#"id="a_{agent}""#))
        .unwrap_or_else(|| panic!("no {agent} in {svg}"));
    let start = svg[..at].rfind("<g ").expect("a group");
    &svg[start..at]
}

/// Everything inside an agent's box.
fn inside<'a>(svg: &'a str, agent: &str) -> &'a str {
    let at = svg.find(&format!(r#"id="a_{agent}""#)).expect("the node");
    let end = svg[at..].find("</g>").expect("closes");
    &svg[at..at + end]
}

/// Whether the route between two nodes is marked as taken.
fn taken(svg: &str, from: &str, to: &str) -> bool {
    let at = svg
        .find(&format!(r#"data-from="{from}" data-to="{to}""#))
        .or_else(|| svg.find(&format!(r#"data-from="{to}" data-to="{from}""#)))
        .unwrap_or_else(|| panic!("no route {from} - {to}"));
    let start = svg[..at].rfind("<g ").expect("a group");
    svg[start..at].contains("travelled")
}

/// A chain that began `minutes` ago: its identifier carries when, as one the Tower minted would.
fn began(minutes: i64) -> ItineraryId {
    let at = Timestamp::now() - SignedDuration::from_mins(minutes);
    let ms = u64::try_from(at.as_millisecond()).expect("after 1970");
    let ulid = ulid::Ulid::from_parts(ms, ulid::Ulid::generate().random());
    ItineraryId::from(format!("itn_{ulid}").as_str())
}

/// Three chains of one workflow at once: one mid-review, one failed, and one waiting for a slot.
fn three_at_once(factory: &Factory) -> (ItineraryId, ItineraryId, ItineraryId) {
    let looping = began(45);
    factory.ran(&looping, "analyst", 40, Outcome::Succeeded, Some(&[]));
    factory.ran(
        &looping,
        "coder",
        30,
        Outcome::Succeeded,
        Some(&["analyst"]),
    );
    factory.ran(
        &looping,
        "reviewer",
        20,
        Outcome::Succeeded,
        Some(&["coder"]),
    );
    factory.running(&looping, "coder", "devforge", "reviewer");

    // Written by a release that did not record senders.
    let failed = began(36);
    factory.ran(&failed, "analyst", 35, Outcome::Succeeded, None);
    factory.ran(&failed, "coder", 25, Outcome::Failed, None);

    let waiting = ItineraryId::generate();
    factory.queued(&waiting);

    (looping, failed, waiting)
}

#[tokio::test]
async fn each_chain_of_one_workflow_is_drawn_by_what_happened_in_it_alone() {
    let factory = Factory::new("alone");
    let (looping, failed, _) = three_at_once(&factory);

    let (status, chain) = json(factory.app(), &format!("/itineraries/{}", looping.as_str())).await;
    assert_eq!(status, StatusCode::OK, "{chain}");
    let svg = chain["map"]["mermaid"].as_str().expect("drawn");
    assert!(node(svg, "coder").contains("running"), "{svg}");
    assert!(
        inside(svg, "coder").contains(">×2</text>"),
        "the coder is on its second pass"
    );
    assert!(node(svg, "analyst").contains("done"));
    assert!(node(svg, "reviewer").contains("done"));
    assert_eq!(node(svg, "helper"), r#"<g class="node agent" "#);
    assert!(taken(svg, "p_devforge", "a_analyst"), "a trigger opened it");
    assert!(taken(svg, "a_analyst", "a_coder"));
    assert!(
        taken(svg, "a_coder", "a_reviewer"),
        "out to review and back"
    );
    assert!(!taken(svg, "a_analyst", "a_helper"));

    assert_eq!(chain["itinerary"]["state"], "working");
    assert_eq!(chain["itinerary"]["running"], serde_json::json!(["coder"]));
    let runs = chain["runs"].as_array().expect("runs");
    let order: Vec<_> = runs.iter().map(|run| run["agent"].clone()).collect();
    assert_eq!(
        order,
        ["analyst", "coder", "reviewer", "coder"],
        "oldest first, the live run last"
    );
    assert_eq!(runs[0]["sent_by"], serde_json::json!([]));
    assert_eq!(runs[2]["sent_by"], serde_json::json!(["coder"]));
    assert_eq!(runs[3]["status"], "running");
    assert_eq!(runs[3]["sent_by"], serde_json::json!(["reviewer"]));

    // The other chain, drawn on the same workflow at the same moment, says something else.
    let (_, other) = json(factory.app(), &format!("/itineraries/{}", failed.as_str())).await;
    let svg = other["map"]["mermaid"].as_str().expect("drawn");
    assert!(node(svg, "coder").contains("failed"), "{svg}");
    assert!(!inside(svg, "coder").contains("×"));
    assert!(node(svg, "analyst").contains("done"));
    assert!(
        !svg.contains("travelled"),
        "runs that did not record their senders light no route rather than a guessed one"
    );
    assert!(
        other["runs"][0]["sent_by"].is_null(),
        "not recorded is null, which is not the empty list of a trigger"
    );
}

#[tokio::test]
async fn a_chain_waiting_for_a_slot_is_listed_and_drawn_with_its_way_in_queued() {
    let factory = Factory::new("waiting");
    let (_, _, waiting) = three_at_once(&factory);

    let (_, chains) = json(factory.app(), "/itineraries?window=last_24h").await;
    let listed = chains["itineraries"]
        .as_array()
        .expect("chains")
        .iter()
        .find(|chain| chain["itinerary_id"] == waiting.as_str())
        .unwrap_or_else(|| panic!("a chain with nothing run yet is still a chain: {chains}"));
    assert_eq!(listed["state"], "working");
    assert_eq!(listed["runs"], 0);
    assert_eq!(listed["pipeline"], "devforge");
    assert_eq!(listed["queued"], serde_json::json!(["analyst"]));

    let (status, chain) = json(factory.app(), &format!("/itineraries/{}", waiting.as_str())).await;
    assert_eq!(status, StatusCode::OK, "{chain}");
    assert_eq!(chain["pending"].as_array().map(Vec::len), Some(1));
    assert_eq!(chain["runs"].as_array().map(Vec::len), Some(0));
    let svg = chain["map"]["mermaid"].as_str().expect("drawn");
    assert!(node(svg, "analyst").contains("queued"), "{svg}");
    assert!(taken(svg, "p_devforge", "a_analyst"));
}

#[tokio::test]
async fn the_workflow_map_counts_an_agent_running_in_several_of_its_chains_at_once() {
    let factory = Factory::new("several");
    let (looping, _, _) = three_at_once(&factory);
    let second = ItineraryId::generate();
    factory.running(&second, "coder", "devforge", "analyst");
    // The same agent, alive in another workflow's chain: not this workflow's to count.
    factory.running(&ItineraryId::generate(), "coder", "nightly", "analyst");
    assert_ne!(looping, second);

    let (_, map) = json(factory.app(), "/graph?pipeline=devforge").await;
    let svg = map["mermaid"].as_str().expect("drawn");
    assert!(node(svg, "coder").contains("running"), "{svg}");
    let coder = inside(svg, "coder");
    assert!(coder.contains(">×2</text>"), "{coder}");
    assert!(coder.contains("2 runs alive now"), "{coder}");

    let (_, whole) = json(factory.app(), "/graph").await;
    let svg = whole["mermaid"].as_str().expect("drawn");
    assert!(
        inside(svg, "coder").contains(">×3</text>"),
        "the whole factory counts all three"
    );
}

#[tokio::test]
async fn a_chain_nothing_knows_about_is_not_found() {
    let factory = Factory::new("unknown");
    three_at_once(&factory);

    let (status, problem) = json(
        factory.app(),
        &format!("/itineraries/{}", ItineraryId::generate().as_str()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{problem}");
}
