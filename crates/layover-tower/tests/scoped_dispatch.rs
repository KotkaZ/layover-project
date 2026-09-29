//! The Tower's side of scoped routes: what it dispatches, parks and abandons for a chain, from the
//! pipeline recorded on queued work — including after a restart, when that record is all it has.

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use common::{echo, factory_text, name, queued};
use layover_core::agent::AgentName;
use layover_core::barrier::Delivery;
use layover_core::config::Config;
use layover_core::flight::ItineraryId;
use layover_core::queue::Queued;
use layover_core::scope::{ChainScope, RouteMap};
use layover_tower::{Barriers, Dispatched, Factory, Refusal};

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
    let config: Config = toml::from_str(&factory_text(echo())).expect("parses");
    Factory::new(config, &temp.0).expect("opens")
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
    let text = format!(
        "{}\n[[routes]]\nfrom = [\"sherlock\", \"wolf\"]\nto = \"bob\"\npipelines = \"follow-up\"\n",
        factory_text(echo())
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
