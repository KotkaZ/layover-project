//! A factory's `[learnings]` numbers reach the Tower: what an agent may propose, how long it lasts,
//! and how many a run is given.

use std::path::Path;
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::flight::{ItineraryId, RunId};
use layover_core::learning::{Impact, Learnings, Proposal};
use layover_core::scope::RouteMap;
use layover_mcp::{Runtime, Session, ToolError};
use layover_store::Journal;
use layover_tower::{FactoryRuntime, Wiring};

mod runs;
use runs::{Temp, factory, human, payloads, sleeps};

const LEARNINGS: &str = "[learnings]\nprovisional_runs = 2\nmax_chars = 40\nmax_per_run = 1\n";

fn text() -> String {
    format!(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "quick"
timeout_sec = 60

[runners.quick]
command = {}

[agents.worker]
prompt = "work"
entry = true

{LEARNINGS}
"#,
        sleeps(0)
    )
}

/// A runtime whose learnings are kept in memory for inspection.
fn runtime(dir: &Path, held: &Arc<Mutex<Learnings>>) -> FactoryRuntime {
    let config: Config = toml::from_str(&text()).expect("parses");
    let writing = Arc::clone(held);
    FactoryRuntime::new(Wiring {
        routes: Arc::new(RouteMap::from_config(&config)),
        config: Arc::new(config),
        hangars: dir.join("hangars"),
        logbook: dir.join("logbook.md"),
        queue: Arc::new(|_| Ok(())),
        book: Arc::new(|_| Ok(())),
        ask: Arc::new(|_| Ok(())),
        file: Arc::new(|_| Ok(())),
        update_learnings: Arc::new(move |change| {
            let mut learnings = writing.lock().expect("lock");
            change(&mut learnings);
            Ok(())
        }),
    })
}

fn session() -> Session {
    Session {
        run: RunId::generate(),
        agent: AgentName::new("worker"),
        itinerary: ItineraryId::generate(),
        hops_remaining: 4,
        pipeline: None,
        flags: std::collections::BTreeMap::new(),
        flight: None,
        within: std::collections::BTreeSet::new(),
        chosen: layover_core::chosen::Chosen::default(),
    }
}

#[test]
fn an_agent_proposes_under_the_factorys_length_and_lifetime() {
    let temp = Temp::new("learning-rules");
    let held = Arc::new(Mutex::new(Learnings::new()));
    let runtime = runtime(&temp.0, &held);

    let error = runtime
        .learn(
            &session(),
            "This sentence is much longer than forty characters allow.",
        )
        .expect_err("too long for this factory");
    assert!(matches!(error, ToolError::BadArguments { .. }));
    assert!(
        error.to_string().contains("at most 40 characters"),
        "{error}"
    );

    runtime
        .learn(&session(), "The e2e suite needs the VPN.")
        .expect("short enough");
    let learnings = held.lock().expect("lock");
    assert_eq!(learnings.len(), 1);
    assert_eq!(
        learnings.all().next().expect("one").runs_left,
        2,
        "it lasts the factory's number of runs"
    );
}

#[test]
fn a_run_is_given_no_more_learnings_than_the_factory_allows() {
    let temp = Temp::new("learning-per-run");
    let factory = factory(&temp, &text());
    let mut learnings = Learnings::new();
    for text in [
        "The workspace needs careful handling.",
        "The manifest needs careful handling.",
        "The registry needs careful handling.",
    ] {
        learnings.propose(&Proposal::new(
            "worker".into(),
            text,
            Impact::Medium,
            Timestamp::now(),
        ));
    }
    Journal::open(temp.0.join(".layover").join("journal"))
        .expect("opens")
        .save_learnings(&learnings)
        .expect("saves");

    factory.drain(vec![human("worker")], |_| {}, |_, _| {});

    let prompt = payloads(&temp.0, "worker").pop().expect("the run's prompt");
    assert_eq!(
        prompt.matches("[provisional]").count(),
        1,
        "max_per_run = 1: {prompt}"
    );
}
