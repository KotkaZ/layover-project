//! Rendezvous joins.
//!
//! A joined agent does not wake until its declared upstreams have arrived. The Tower parks
//! flights rather than parking processes, which is what makes fan-in possible without a blocking
//! request/response mode.
//!
//! Three behaviours here are subtle:
//!
//! - **Scope.** A barrier constrains the upstreams it names and nobody else. The same agent may
//!   also sit behind ordinary unjoined edges — including a human entry point — and a flight
//!   arriving on one of those wakes it immediately without touching the barrier. A join declares
//!   *which inputs an agent needs together*, not *when an agent is allowed to run*.
//! - **Reset.** A second delivery from the same upstream discards partial state, so a stale
//!   result from a sibling branch cannot satisfy the barrier alongside a fresh one.
//! - **Abandonment.** A barrier whose missing upstreams can no longer be reached by any live run
//!   is dead, and must be abandoned rather than parked forever.

use std::collections::{BTreeMap, BTreeSet};

use crate::agent::AgentName;
use crate::flight::{Flight, ItineraryId};
use crate::graph::JoinSpec;
use crate::graph::RouteGraph;
use crate::route::Join;

/// Identifies a barrier within the Tower.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BarrierKey {
    /// The chain the barrier belongs to.
    pub itinerary: ItineraryId,
    /// The agent being guarded.
    pub to: AgentName,
}

impl BarrierKey {
    /// Builds a key.
    #[must_use]
    pub fn new(itinerary: ItineraryId, to: AgentName) -> Self {
        Self { itinerary, to }
    }
}

/// The outcome of delivering a flight to a joined agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// The flight was parked; the agent stays asleep.
    Parked {
        /// Upstreams still outstanding.
        waiting_for: Vec<AgentName>,
    },
    /// The condition is met. The agent should be spawned exactly once with these flights.
    Ready(Vec<Flight>),
    /// The sender is not a declared upstream, so the barrier does not apply.
    ///
    /// The flight bypasses the barrier and wakes the agent on its own. It is not an error: the
    /// route check has already established that the edge exists, and a barrier only speaks for
    /// the upstreams it names. This is what lets a joined agent also be an entry point.
    Direct(Box<Flight>),
    /// The barrier already released for this dispatch wave, and this upstream arrived after.
    ///
    /// Only `join = "any"` produces this: it releases on the first arrival, so every other
    /// upstream in the same wave is necessarily late. Dropping the flight is the whole point of
    /// `any` — the alternative is waking the agent once per upstream, which for a publisher means
    /// one pull request per straggler.
    ///
    /// Returned rather than silently discarded so the Tower can record that work was superseded.
    Late(Box<Flight>),
}

/// A rendezvous barrier holding flights until its condition is met.
#[derive(Debug, Clone)]
pub struct Barrier {
    required: BTreeSet<AgentName>,
    join: Join,
    parked: BTreeMap<AgentName, Flight>,
    /// Upstreams that have arrived in the current dispatch wave, parked or already consumed.
    ///
    /// Distinct from `parked`, which is emptied when the barrier releases. Without this an `any`
    /// barrier forgets it ever fired.
    seen: BTreeSet<AgentName>,
    /// Whether this wave has already woken the agent.
    released: bool,
}

impl Barrier {
    /// Creates a barrier from a route's rendezvous condition.
    #[must_use]
    pub fn from_spec(spec: &JoinSpec) -> Self {
        Self {
            required: spec.upstreams.clone(),
            join: spec.join,
            parked: BTreeMap::new(),
            seen: BTreeSet::new(),
            released: false,
        }
    }

    /// Delivers a flight to the barrier.
    ///
    /// A flight from a sender the barrier does not name is returned as [`Delivery::Direct`] and
    /// leaves parked state untouched, so an agent behind a join can still be triggered by a human
    /// or by an unjoined peer.
    ///
    /// A second delivery from an upstream that has already reported starts a **new dispatch
    /// wave**: partial state is discarded and every upstream must deliver again. This is what
    /// stops a stale verdict from before a failure loop-back being combined with a fresh one, and
    /// it is also what lets a loop re-run: the barrier is reusable, but only deliberately.
    ///
    /// Within one wave the agent is woken at most once. An upstream arriving after an `any`
    /// barrier has fired is [`Delivery::Late`].
    pub fn deliver(&mut self, flight: Flight) -> Delivery {
        let Some(sender) = flight.from.agent().cloned() else {
            return Delivery::Direct(Box::new(flight));
        };

        if !self.required.contains(&sender) {
            return Delivery::Direct(Box::new(flight));
        }

        // An upstream reporting twice is the signal that a new wave has begun — under `all`
        // because the fan-out was re-dispatched, and under `any` because the loop came round.
        if self.seen.contains(&sender) {
            self.parked.clear();
            self.seen.clear();
            self.released = false;
        }
        self.seen.insert(sender.clone());

        if self.released {
            return Delivery::Late(Box::new(flight));
        }

        self.parked.insert(sender, flight);

        let complete = match self.join {
            Join::Any => true,
            Join::All => self.parked.len() == self.required.len(),
        };

        if complete {
            self.released = true;
            Delivery::Ready(std::mem::take(&mut self.parked).into_values().collect())
        } else {
            Delivery::Parked {
                waiting_for: self.waiting_for(),
            }
        }
    }

    /// Returns `true` when this wave has already woken the agent.
    #[must_use]
    pub fn has_released(&self) -> bool {
        self.released
    }

    /// Upstreams that have not yet delivered.
    #[must_use]
    pub fn waiting_for(&self) -> Vec<AgentName> {
        self.required
            .iter()
            .filter(|name| !self.parked.contains_key(*name))
            .cloned()
            .collect()
    }

    /// Returns `true` if any live run could still satisfy this barrier.
    ///
    /// When this is `false` the barrier is dead: no process remains that could deliver the
    /// missing upstreams, so the itinerary should be marked stalled rather than left parked.
    #[must_use]
    pub fn is_reachable(&self, graph: &RouteGraph, live: &BTreeSet<AgentName>) -> bool {
        let missing = self.waiting_for();
        if missing.is_empty() {
            return true;
        }

        let reachable = graph.reachable_from(live);
        match self.join {
            Join::All => missing.iter().all(|name| reachable.contains(name)),
            Join::Any => missing.iter().any(|name| reachable.contains(name)),
        }
    }

    /// Number of upstreams currently parked.
    #[must_use]
    pub fn parked_count(&self) -> usize {
        self.parked.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::flight::Origin;

    #[test]
    fn an_any_barrier_wakes_the_agent_once_however_many_upstreams_arrive() {
        // `Delivery::Ready` promises the agent is spawned exactly once. Before this, an `any`
        // barrier released on the first arrival, cleared its parked state, and then released
        // again on the second -- so a two-upstream `any` into a publisher opened two pull
        // requests, with Hops, Fuel and the run cap all satisfied because both were ordinary
        // first runs. Manual recovery could not help: neither was a recovery.
        let mut barrier = barrier_any();

        assert!(matches!(
            barrier.deliver(flight_from("probe_a", "first")),
            Delivery::Ready(_)
        ));
        assert!(barrier.has_released());

        match barrier.deliver(flight_from("probe_b", "second")) {
            Delivery::Late(flight) => assert_eq!(flight.body, "second"),
            other => panic!("the straggler must not wake the agent again: {other:?}"),
        }
    }

    #[test]
    fn an_any_barrier_can_still_fire_again_on_the_next_time_round_the_loop() {
        // Late must not mean dead. A repeat from an upstream that already reported is the signal
        // that a new dispatch wave has begun, which is what makes a barrier inside a loop usable.
        let mut barrier = barrier_any();
        barrier.deliver(flight_from("probe_a", "first"));
        barrier.deliver(flight_from("probe_b", "late"));

        match barrier.deliver(flight_from("probe_a", "next time round")) {
            Delivery::Ready(flights) => {
                assert_eq!(flights.len(), 1);
                assert_eq!(flights[0].body, "next time round");
            }
            other => panic!("a new wave should release: {other:?}"),
        }
    }

    #[test]
    fn an_all_barrier_also_wakes_the_agent_only_once_per_wave() {
        let mut barrier = barrier_all();
        barrier.deliver(flight_from("probe_a", "a"));

        assert!(matches!(
            barrier.deliver(flight_from("probe_b", "b")),
            Delivery::Ready(_)
        ));
        assert!(barrier.has_released());
    }

    fn flight_from(sender: &str, body: &str) -> Flight {
        Flight::new(
            ItineraryId::generate(),
            Origin::Agent(sender.into()),
            "collector".into(),
            body,
            5,
        )
    }

    fn barrier_any() -> Barrier {
        Barrier {
            required: ["probe_a".into(), "probe_b".into()].into_iter().collect(),
            join: Join::Any,
            parked: BTreeMap::new(),
            seen: BTreeSet::new(),
            released: false,
        }
    }

    fn barrier_all() -> Barrier {
        Barrier {
            required: ["probe_a".into(), "probe_b".into()].into_iter().collect(),
            join: Join::All,
            parked: BTreeMap::new(),
            seen: BTreeSet::new(),
            released: false,
        }
    }

    #[test]
    fn parks_until_every_upstream_arrives() {
        let mut barrier = barrier_all();

        let first = barrier.deliver(flight_from("probe_a", "a"));

        assert_eq!(
            first,
            Delivery::Parked {
                waiting_for: vec!["probe_b".into()]
            }
        );
    }

    #[test]
    fn releases_once_with_every_parked_flight() {
        let mut barrier = barrier_all();
        let _ = barrier.deliver(flight_from("probe_a", "a"));

        let Delivery::Ready(flights) = barrier.deliver(flight_from("probe_b", "b")) else {
            panic!("barrier should release once both upstreams arrive");
        };

        assert_eq!(flights.len(), 2);
        assert_eq!(barrier.parked_count(), 0, "barrier drains on release");
    }

    #[test]
    fn join_any_releases_on_the_first_arrival() {
        let mut barrier = Barrier {
            required: ["probe_a".into(), "probe_b".into()].into_iter().collect(),
            join: Join::Any,
            parked: BTreeMap::new(),
            seen: BTreeSet::new(),
            released: false,
        };

        let Delivery::Ready(flights) = barrier.deliver(flight_from("probe_a", "a")) else {
            panic!("join = any should release immediately");
        };

        assert_eq!(flights.len(), 1);
    }

    #[test]
    fn a_repeat_delivery_discards_stale_siblings() {
        let mut barrier = barrier_all();
        let _ = barrier.deliver(flight_from("probe_b", "stale verdict"));
        let _ = barrier.deliver(flight_from("probe_a", "first attempt"));
        // Both have now delivered, so the barrier already released; re-park to model a re-run.
        let _ = barrier.deliver(flight_from("probe_a", "first attempt"));

        let outcome = barrier.deliver(flight_from("probe_a", "second attempt"));

        assert_eq!(
            outcome,
            Delivery::Parked {
                waiting_for: vec!["probe_b".into()]
            },
            "a repeat delivery must discard partial state, not complete the barrier"
        );
    }

    #[test]
    fn a_sender_outside_the_join_is_delivered_directly() {
        let mut barrier = barrier_all();

        let outcome = barrier.deliver(flight_from("stranger", "hello"));

        assert!(matches!(outcome, Delivery::Direct(_)));
        assert_eq!(barrier.parked_count(), 0);
    }

    #[test]
    fn a_human_flight_is_delivered_directly() {
        let mut barrier = barrier_all();
        let flight = Flight::new(
            ItineraryId::generate(),
            Origin::Human,
            "collector".into(),
            "go",
            5,
        );

        assert!(matches!(barrier.deliver(flight), Delivery::Direct(_)));
    }

    #[test]
    fn a_direct_flight_leaves_parked_state_intact() {
        // A joined agent may also be an entry point. Waking it by the front door must not discard
        // the half-collected rendezvous, or the upstream that already reported would be lost.
        let mut barrier = barrier_all();
        let _ = barrier.deliver(flight_from("probe_a", "a"));

        let _ = barrier.deliver(flight_from("stranger", "unrelated work"));

        assert_eq!(barrier.parked_count(), 1);
        assert_eq!(barrier.waiting_for(), vec![AgentName::from("probe_b")]);

        let Delivery::Ready(flights) = barrier.deliver(flight_from("probe_b", "b")) else {
            panic!("the barrier should still complete normally");
        };
        assert_eq!(flights.len(), 2);
    }

    fn pipeline_graph() -> RouteGraph {
        let config = Config::from_toml(
            r#"
            [[routes]]
            from = "planner"
            to = ["probe_a", "probe_b"]

            [[routes]]
            from = ["probe_a", "probe_b"]
            to = "collector"
            join = "all"

            [[routes]]
            from = "probe_a"
            to = "planner"
            "#,
            "test.toml",
        )
        .expect("config parses");
        RouteGraph::from_config(&config)
    }

    #[test]
    fn a_live_upstream_keeps_the_barrier_alive() {
        let mut barrier = barrier_all();
        let _ = barrier.deliver(flight_from("probe_a", "a"));
        let live = ["probe_b".into()].into_iter().collect();

        assert!(barrier.is_reachable(&pipeline_graph(), &live));
    }

    #[test]
    fn an_upstream_reachable_from_a_live_run_keeps_it_alive() {
        let mut barrier = barrier_all();
        let _ = barrier.deliver(flight_from("probe_a", "a"));
        // planner is live and can still reach probe_b.
        let live = ["planner".into()].into_iter().collect();

        assert!(barrier.is_reachable(&pipeline_graph(), &live));
    }

    #[test]
    fn a_barrier_no_live_run_can_reach_is_abandoned() {
        // The loop-back hazard: probe_a diverted back to the planner instead of the collector,
        // and nothing left running can deliver probe_b.
        let mut barrier = barrier_all();
        let _ = barrier.deliver(flight_from("probe_a", "a"));
        let live = ["collector".into()].into_iter().collect();

        assert!(
            !barrier.is_reachable(&pipeline_graph(), &live),
            "a barrier that nothing can satisfy must be abandoned, not parked forever"
        );
    }

    #[test]
    fn a_barrier_with_nothing_live_is_abandoned() {
        let mut barrier = barrier_all();
        let _ = barrier.deliver(flight_from("probe_a", "a"));

        assert!(!barrier.is_reachable(&pipeline_graph(), &BTreeSet::new()));
    }
}
