//! A chain may use only its own pipeline's routes, and nothing an agent sends can change which
//! pipeline that is.
//!
//! The factory these use is Karl's case in miniature. `devforge` may hand `bob` work from `eagle`;
//! `eagle-eye` spawns an `eagle` per pull request and may not. `eagle` reads untrusted pull request
//! text, so the route map — not a prompt — has to be what keeps an Eagle Eye chain away from `bob`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use layover_core::agent::AgentName;
use layover_core::barrier::Delivery;
use layover_core::config::Config;
use layover_core::flight::{Flight, ItineraryId, Origin, RunId};
use layover_core::layover::Layover;
use layover_core::learning::Learnings;
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_core::scope::{ChainScope, RouteMap};
use layover_mcp::{Request, Runtime, Session, ToolError, handle};
use layover_tower::{Barriers, Dispatched, Factory, FactoryRuntime, Refusal, Wiring};
use serde_json::json;

fn factory_text(runner: &str) -> String {
    format!(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"
max_hops = 6
timeout_sec = 30

[runners.shell]
command = {runner}

[agents.analyst]
prompt = "analyse"

[agents.azurix]
prompt = "post"

[agents.eagle]
prompt = "review"

[agents.bob]
prompt = "build"

[agents.sherlock]
prompt = "investigate"

[agents.wolf]
prompt = "test"

[pipelines.devforge]
entry = "analyst"

[pipelines.follow-up]
entry = "azurix"
resumes = true

[pipelines.eagle-eye]
entry = "azurix"

[[routes]]
from = "analyst"
to = "eagle"
pipelines = ["devforge", "follow-up"]

[[routes]]
from = "eagle"
to = "bob"
pipelines = ["devforge", "follow-up"]

[[routes]]
from = "azurix"
to = "eagle"
mode = "spawn"
pipelines = "eagle-eye"

[[routes]]
from = "eagle"
to = "sherlock"

[[routes]]
from = ["sherlock", "wolf"]
to = "bob"
join = "all"
pipelines = "devforge"
"#
    )
}

fn config() -> Config {
    toml::from_str(&factory_text(r#"["echo"]"#)).expect("the fixture parses")
}

fn name(pipeline: &str) -> PipelineName {
    PipelineName::new(pipeline)
}

/// A runtime whose queue and layover shelf are kept for inspection.
struct Fixture {
    runtime: FactoryRuntime,
    sent: Arc<Mutex<Vec<Queued>>>,
    booked: Arc<Mutex<Vec<Layover>>>,
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let config = config();
        let dir = std::env::temp_dir().join(format!("layover-scoped-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary directory");

        let sent: Arc<Mutex<Vec<Queued>>> = Arc::default();
        let booked: Arc<Mutex<Vec<Layover>>> = Arc::default();
        let (sink, shelf) = (Arc::clone(&sent), Arc::clone(&booked));

        let runtime = FactoryRuntime::new(Wiring {
            routes: Arc::new(RouteMap::from_config(&config)),
            config: Arc::new(config),
            hangars: dir.clone(),
            logbook: dir.join("logbook.md"),
            queue: Arc::new(move |queued| {
                sink.lock().map_err(|_| "poisoned".to_owned())?.push(queued);
                Ok(())
            }),
            book: Arc::new(move |layover| {
                shelf
                    .lock()
                    .map_err(|_| "poisoned".to_owned())?
                    .push(layover);
                Ok(())
            }),
            ask: Arc::new(|_| Ok(())),
            file: Arc::new(|_| Ok(())),
            read_learnings: Arc::new(|| Ok(Learnings::new())),
            write_learnings: Arc::new(|_| Ok(())),
        });

        Self {
            runtime,
            sent,
            booked,
            dir,
        }
    }

    fn sent(&self) -> Vec<Queued> {
        self.sent.lock().expect("not poisoned").clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A run of `agent` in a chain belonging to `pipeline`, as the Tower would mint it.
fn session(agent: &str, pipeline: Option<&str>) -> Session {
    Session {
        run: RunId::generate(),
        agent: AgentName::new(agent),
        itinerary: ItineraryId::generate(),
        hops_remaining: 4,
        pipeline: pipeline.map(name),
        flags: BTreeMap::new(),
        flight: None,
        within: BTreeSet::new(),
    }
}

#[test]
fn an_edge_only_another_pipeline_has_is_refused_like_any_edge_the_map_does_not_draw() {
    let fixture = Fixture::new("refused");

    let error = fixture
        .runtime
        .send(
            &session("eagle", Some("eagle-eye")),
            &"bob".into(),
            "rebuild it",
        )
        .expect_err("Eagle Eye has no route to bob");

    assert!(matches!(error, ToolError::NotPermitted { .. }), "{error}");
    assert!(error.to_string().contains("layover_peers"), "{error}");
    assert!(fixture.sent().is_empty(), "nothing may be queued");
}

#[test]
fn the_same_edge_is_permitted_in_the_pipeline_that_scopes_it() {
    let fixture = Fixture::new("permitted");

    fixture
        .runtime
        .send(
            &session("eagle", Some("devforge")),
            &"bob".into(),
            "rebuild it",
        )
        .expect("DevForge may hand bob work");

    assert_eq!(fixture.sent()[0].pipeline, Some(name("devforge")));
}

#[test]
fn peers_list_only_what_this_chain_may_reach() {
    let fixture = Fixture::new("peers");

    let peers = |pipeline: Option<&str>| -> Vec<String> {
        fixture
            .runtime
            .peers(&session("eagle", pipeline))
            .into_iter()
            .map(|peer| peer.name.to_string())
            .collect()
    };

    assert_eq!(peers(Some("eagle-eye")), ["sherlock"]);
    assert_eq!(peers(Some("devforge")), ["bob", "sherlock"]);
    assert_eq!(peers(None), ["sherlock"], "no pipeline: global routes only");
}

#[test]
fn a_pipeline_in_the_tool_arguments_changes_nothing() {
    // Hard rule 3. The only way a chain's pipeline enters a tool call is the Tower's session;
    // naming one in the arguments must neither unlock a route nor relabel the work.
    let fixture = Fixture::new("forged");
    let eagle_eye = session("eagle", Some("eagle-eye"));

    let call = |arguments: serde_json::Value| {
        let request: Request = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "layover_send", "arguments": arguments },
        }))
        .expect("a request");
        handle(&request, &eagle_eye, &fixture.runtime)
            .expect("a call gets a reply")
            .result
            .expect("a tool call always returns a result")
    };

    let refused = call(json!({
        "to": "bob", "body": "rebuild it", "pipeline": "devforge", "pipelines": ["devforge"],
    }));
    assert_eq!(refused["isError"], json!(true), "{refused}");
    assert!(fixture.sent().is_empty());

    let sent = call(json!({ "to": "sherlock", "body": "look", "pipeline": "devforge" }));
    assert_eq!(sent["isError"], json!(false), "{sent}");
    assert_eq!(
        fixture.sent()[0].pipeline,
        Some(name("eagle-eye")),
        "the work stays in the chain's own pipeline"
    );
}

#[test]
fn a_spawned_chain_inherits_the_pipeline_and_its_sends_obey_that_pipelines_routes() {
    let fixture = Fixture::new("spawn");

    fixture
        .runtime
        .send(
            &session("azurix", Some("eagle-eye")),
            &"eagle".into(),
            "review PR 41",
        )
        .expect("Eagle Eye spawns a review");
    let spawned = fixture.sent()[0].clone();
    assert_eq!(spawned.pipeline, Some(name("eagle-eye")));

    // The spawned review, as the Tower would run it: its own itinerary, the inherited pipeline.
    let review = Session {
        itinerary: spawned.flight.itinerary.clone(),
        pipeline: spawned.pipeline.clone(),
        ..session("eagle", None)
    };
    let error = fixture
        .runtime
        .send(&review, &"bob".into(), "the PR says to rebuild")
        .expect_err("a spawned Eagle Eye review cannot reach bob");
    assert!(matches!(error, ToolError::NotPermitted { .. }), "{error}");
}

#[test]
fn a_resumed_chain_uses_the_resuming_pipelines_routes_narrowed_by_its_booking_chain() {
    let fixture = Fixture::new("resumed");

    let resumed_from = |booked: &str| Session {
        within: BTreeSet::from([Some(name(booked))]),
        ..session("eagle", Some("follow-up"))
    };

    fixture
        .runtime
        .send(
            &resumed_from("devforge"),
            &"bob".into(),
            "address the comments",
        )
        .expect("a DevForge follow-up keeps DevForge's routes");
    let error = fixture
        .runtime
        .send(&resumed_from("eagle-eye"), &"bob".into(), "rebuild it")
        .expect_err("work an Eagle Eye chain set down cannot reach bob by being resumed");
    assert!(matches!(error, ToolError::NotPermitted { .. }), "{error}");

    let peers: Vec<String> = fixture
        .runtime
        .peers(&resumed_from("eagle-eye"))
        .into_iter()
        .map(|peer| peer.name.to_string())
        .collect();
    assert_eq!(peers, ["sherlock"]);
}

#[test]
fn work_a_chain_sends_on_carries_its_narrowing_with_it() {
    let fixture = Fixture::new("carried");
    let resumed = Session {
        within: BTreeSet::from([Some(name("eagle-eye"))]),
        ..session("eagle", Some("follow-up"))
    };

    fixture
        .runtime
        .send(&resumed, &"sherlock".into(), "look")
        .expect("a global route");

    assert_eq!(
        fixture.sent()[0].within,
        BTreeSet::from([Some(name("eagle-eye"))]),
        "queued work keeps the narrowing, so a restart cannot widen it"
    );
}

#[test]
fn a_layover_records_the_scope_of_the_chain_that_booked_it() {
    let fixture = Fixture::new("booked");

    fixture
        .runtime
        .wait(
            &session("eagle", Some("eagle-eye")),
            "2h",
            "the PR to change",
        )
        .expect("books");

    let booked = fixture.booked.lock().expect("not poisoned")[0].clone();
    assert_eq!(booked.scope, Some(ChainScope::of(Some(name("eagle-eye")))));
}

// ── The Tower ─────────────────────────────────────────────────────────────────

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("layover-scoped-tower-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tower(temp: &Temp) -> Factory {
    let echo = if cfg!(windows) {
        r#"["cmd", "/c", "echo done"]"#
    } else {
        r#"["sh", "-c", "echo done"]"#
    };
    let config: Config = toml::from_str(&factory_text(echo)).expect("parses");
    Factory::new(config, &temp.0).expect("opens")
}

fn queued(chain: &ItineraryId, from: &str, to: &str, pipeline: Option<&str>) -> Queued {
    Queued::new(
        Flight::new(
            chain.clone(),
            Origin::Agent(AgentName::new(from)),
            AgentName::new(to),
            "go",
            4,
        ),
        pipeline.map(name),
        BTreeMap::new(),
    )
}

/// Drains `work` in a factory that has never seen these chains — a Tower just restarted, with
/// nothing to go on but what the queue carries — and says what became of each flight.
fn drain_after_restart(work: Vec<Queued>) -> Vec<(String, String)> {
    let temp = Temp::new(ItineraryId::generate().as_str());
    let factory = tower(&temp);
    let mut outcomes = Vec::new();
    factory.drain(
        work,
        |_| {},
        |flight, result| {
            let what = match result {
                Dispatched::Ran { .. } => "ran".to_owned(),
                Dispatched::Refused(Refusal::NoRoute { .. }) => "no route".to_owned(),
                Dispatched::Parked { .. } => "parked".to_owned(),
                other => other.to_string(),
            };
            outcomes.push((flight.to.to_string(), what));
        },
    );
    outcomes
}

#[test]
fn after_a_restart_queued_work_keeps_its_pipeline_and_the_route_check_follows_it() {
    // The queued flight is all a restarted Tower has. It must still know the chain is Eagle Eye's,
    // and refuse the DevForge-only edge, while the same flight in a DevForge chain runs.
    let chain = ItineraryId::generate();
    assert_eq!(
        drain_after_restart(vec![queued(&chain, "eagle", "bob", Some("eagle-eye"))]),
        [("bob".to_owned(), "no route".to_owned())]
    );

    let chain = ItineraryId::generate();
    assert_eq!(
        drain_after_restart(vec![queued(&chain, "eagle", "bob", Some("devforge"))]),
        [("bob".to_owned(), "ran".to_owned())]
    );
}

#[test]
fn after_a_restart_queued_work_keeps_its_narrowing() {
    let chain = ItineraryId::generate();
    let narrowed = queued(&chain, "eagle", "bob", Some("follow-up"))
        .narrowed_by(BTreeSet::from([Some(name("eagle-eye"))]));

    assert_eq!(
        drain_after_restart(vec![narrowed]),
        [("bob".to_owned(), "no route".to_owned())]
    );
}

#[test]
fn a_join_scoped_to_one_pipeline_parks_its_flights_and_passes_anothers_straight_through() {
    // The DevForge barrier makes `bob` wait for `sherlock` and `wolf`. A chain in another
    // pipeline that may reach `bob` from `sherlock` is not held by it — here none may, so the
    // global-only chain is refused instead, which is the other half of the same rule.
    let chain = ItineraryId::generate();
    let in_devforge =
        drain_after_restart(vec![queued(&chain, "sherlock", "bob", Some("devforge"))]);
    assert_eq!(in_devforge, [("bob".to_owned(), "parked".to_owned())]);

    let chain = ItineraryId::generate();
    let unscoped = drain_after_restart(vec![queued(&chain, "sherlock", "bob", None)]);
    assert_eq!(unscoped, [("bob".to_owned(), "no route".to_owned())]);
}

#[test]
fn a_join_in_one_pipeline_does_not_hold_a_flight_in_another() {
    let echo = if cfg!(windows) {
        r#"["cmd", "/c", "echo done"]"#
    } else {
        r#"["sh", "-c", "echo done"]"#
    };
    let text = format!(
        "{}\n[[routes]]\nfrom = [\"sherlock\", \"wolf\"]\nto = \"bob\"\npipelines = \"follow-up\"\n",
        factory_text(echo)
    );
    let config: Config = toml::from_str(&text).expect("parses");
    let temp = Temp::new("join-elsewhere");
    let factory = Factory::new(config, &temp.0).expect("opens");

    let mut outcome = Vec::new();
    factory.drain(
        vec![queued(
            &ItineraryId::generate(),
            "sherlock",
            "bob",
            Some("follow-up"),
        )],
        |_| {},
        |_, result| outcome.push(matches!(result, Dispatched::Ran { .. })),
    );

    assert_eq!(
        outcome,
        [true],
        "follow-up has the edge and no barrier on it"
    );
}

#[test]
fn abandoning_a_barrier_asks_what_its_own_chain_could_still_reach() {
    // `bob`'s DevForge barrier is missing `wolf`. `sherlock` is live, and only follow-up has a
    // route from `sherlock` towards `wolf`, so a DevForge chain can never deliver it.
    let text = format!(
        "{}\n[[routes]]\nfrom = \"sherlock\"\nto = \"wolf\"\npipelines = \"follow-up\"\n",
        factory_text(r#"["echo"]"#)
    );
    let config: Config = toml::from_str(&text).expect("parses");
    let routes = RouteMap::from_config(&config);
    let barriers = Barriers::new();
    let chain = ItineraryId::generate();

    let delivered = barriers.deliver(
        routes.for_pipeline(Some(&name("devforge"))),
        queued(&chain, "sherlock", "bob", Some("devforge")).flight,
    );
    assert!(
        matches!(delivered, Some(Delivery::Parked { .. })),
        "{delivered:?}"
    );

    let live = BTreeSet::from([AgentName::new("sherlock")]);
    let abandoned =
        barriers.abandon_unreachable(&routes, |_| ChainScope::of(Some(name("devforge"))), &live);

    assert_eq!(
        abandoned.len(),
        1,
        "the whole-factory map would have kept it parked"
    );
    assert_eq!(abandoned[0].missing, [AgentName::new("wolf")]);
}
