//! A help request remembers the chain that asked, and a reply continues that chain's work.
//!
//! The request is filed by the agent, through the runtime the MCP endpoint calls, so what it
//! records comes from the run's own session — the Tower's account of the chain, not the agent's.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::flight::{ItineraryId, RunId};
use layover_core::help::{Blocker, HelpRequest, reply};
use layover_core::learning::Learnings;
use layover_core::pipeline::PipelineName;
use layover_core::run::RunRecord;
use layover_core::scope::{ChainScope, RouteMap};
use layover_mcp::{Runtime, Session};
use layover_tower::{Factory, FactoryRuntime, Wiring};

mod runs;
use runs::{Temp, history};

fn config(runner: &str) -> Config {
    toml::from_str(&format!(
        r#"
[layover]
work_dir = "work"
prompt_dir = "prompts"

[defaults]
runner = "shell"
timeout_sec = 30

[runners.shell]
command = {runner}

[agents.analyst]
prompt = "specify"

[agents.bob]
prompt_file = "bob.md"

[pipelines.devforge]
entry = "analyst"

[pipelines.devforge.flags]
publish_pr = {{ default = false }}

[pipelines.follow-up]
entry = "analyst"

[[routes]]
from = "analyst"
to = "bob"
"#
    ))
    .expect("parses")
}

fn quick() -> &'static str {
    if cfg!(windows) {
        r#"["cmd", "/c", "echo done"]"#
    } else {
        r#"["sh", "-c", "echo done"]"#
    }
}

/// A runtime whose help requests are kept for inspection.
fn runtime(temp: &Temp, asked: &Arc<Mutex<Vec<HelpRequest>>>) -> FactoryRuntime {
    let config = config(quick());
    let sink = Arc::clone(asked);
    FactoryRuntime::new(Wiring {
        routes: Arc::new(RouteMap::from_config(&config)),
        config: Arc::new(config),
        hangars: temp.0.join("hangars"),
        logbook: temp.0.join("logbook.md"),
        queue: Arc::new(|_| Ok(())),
        book: Arc::new(|_| Ok(())),
        ask: Arc::new(move |request| {
            sink.lock().expect("lock").push(request);
            Ok(())
        }),
        file: Arc::new(|_| Ok(())),
        update_learnings: Arc::new(|change| {
            change(&mut Learnings::new());
            Ok(())
        }),
    })
}

fn session(pipeline: Option<&str>, flags: &[(&str, bool)]) -> Session {
    Session {
        run: RunId::generate(),
        agent: AgentName::new("bob"),
        itinerary: ItineraryId::generate(),
        hops_remaining: 4,
        pipeline: pipeline.map(PipelineName::new),
        flags: flags
            .iter()
            .map(|(name, on)| ((*name).to_owned(), *on))
            .collect(),
        flight: None,
        within: BTreeSet::from([Some(PipelineName::new("follow-up"))]),
    }
}

#[test]
fn a_help_request_records_the_workflow_routes_and_flags_of_the_chain_that_asked() {
    let temp = Temp::new("help-records");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime(&temp, &asked);

    let asking = session(Some("devforge"), &[("publish_pr", true)]);
    runtime
        .help(
            &asking,
            Blocker::Decision,
            "spec needs 3 decisions",
            "which policy?",
            true,
        )
        .expect("files");

    let request = asked.lock().expect("lock")[0].clone();
    assert!(request.records_its_chain());
    assert_eq!(
        request.scope,
        Some(ChainScope::new(
            Some(PipelineName::new("devforge")),
            asking.within.clone()
        ))
    );
    assert_eq!(
        request.flags,
        BTreeMap::from([("publish_pr".to_owned(), true)])
    );
}

#[test]
fn a_chain_no_workflow_opened_records_no_flags_of_its_own() {
    // Its session carries every declared flag at its default, which are nobody's choices.
    let temp = Temp::new("help-bare");
    let asked = Arc::new(Mutex::new(Vec::new()));
    let runtime = runtime(&temp, &asked);

    runtime
        .help(
            &session(None, &[("publish_pr", false)]),
            Blocker::Access,
            "no token",
            "401",
            false,
        )
        .expect("files");

    let request = asked.lock().expect("lock")[0].clone();
    assert!(request.records_its_chain());
    assert!(request.flags.is_empty());
}

fn payload_of(temp: &Temp, agent: &str) -> String {
    let dir: PathBuf = temp.0.join(".layover").join("hangars").join(agent);
    std::fs::read_dir(&dir)
        .expect("ran")
        .flatten()
        .find_map(|entry| std::fs::read_to_string(entry.path().join("prompt.md")).ok())
        .expect("a payload")
}

#[test]
fn a_reply_runs_with_the_chains_flags_told_a_person_wrote_it_and_linked_to_the_chain_it_continues()
{
    let temp = Temp::new("reply-runs");
    let config = config(quick());
    let earlier = ItineraryId::generate();
    let request = HelpRequest::new(
        AgentName::new("bob"),
        RunId::generate(),
        earlier.clone(),
        Blocker::Decision,
        "spec needs 3 decisions",
        "which policy?",
        jiff::Timestamp::now(),
    )
    .fatal()
    .raised_in(
        ChainScope::new(Some(PipelineName::new("devforge")), BTreeSet::new()),
        BTreeMap::from([("publish_pr".to_owned(), true)]),
    );
    let queued = reply::continuation(&config, &[request], "Use exponential back-off.", None, None)
        .expect("continues");
    let chain = queued.flight.itinerary.clone();

    let prompts = temp.0.join("prompts");
    std::fs::create_dir_all(&prompts).expect("dirs");
    std::fs::write(
        prompts.join("bob.md"),
        "Build it.\n@include(publish_pr) publish.md\n@include(!publish_pr) hold.md\n",
    )
    .expect("writes");
    std::fs::write(prompts.join("publish.md"), "You may open the pull request.").expect("writes");
    std::fs::write(prompts.join("hold.md"), "Do not open a pull request.").expect("writes");

    let factory = Factory::new(config, &temp.0).expect("opens");
    let mut results = Vec::new();
    factory.drain(
        vec![queued],
        |_| {},
        |_, result| results.push(result.to_string()),
    );
    assert_eq!(results.len(), 1, "{results:?}");

    let told = payload_of(&temp, "bob");
    assert!(told.contains("A person sent this"), "{told}");
    assert!(told.contains("In reply to your help request"), "{told}");
    assert!(told.contains("Use exponential back-off."), "{told}");
    assert!(
        told.contains("You may open the pull request.") && !told.contains("Do not open"),
        "composed with the chain's flags, not the defaults: {told}"
    );

    let ran: Vec<RunRecord> = history(&temp.0);
    assert_eq!(ran[0].itinerary, chain);
    assert_eq!(
        ran[0].flags,
        BTreeMap::from([("publish_pr".to_owned(), true)])
    );
    assert_eq!(ran[0].continues.as_ref(), Some(&earlier));
}
