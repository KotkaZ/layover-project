//! Barriers a running factory is holding, and what happens to a flight that meets one.
//!
//! # Why flights are parked and processes are not
//!
//! A joined agent needs several inputs together. The obvious implementation — start it and let it
//! block until the rest arrive — costs a live process per waiting branch, and those processes are
//! agent CLIs: they have context windows, they cost money per minute in some pricing models, and a
//! supervisor that dies leaves them orphaned. Parking the *flight* costs a map entry.
//!
//! It also makes the wait durable. A parked flight is data, so it survives being written down; a
//! blocked process is not.
//!
//! # Why a dead barrier is abandoned rather than left
//!
//! A barrier waiting for an upstream that no live run can still produce will wait forever, holding
//! work somebody asked for. Silent permanent stalling is the worst outcome in this system — worse
//! than a failure, which at least says something happened. So after every pass the barriers are
//! checked against what can still be reached, and the ones that cannot complete are given up and
//! reported.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use layover_core::agent::AgentName;
use layover_core::barrier::{Barrier, BarrierKey, Delivery};
use layover_core::flight::Flight;
use layover_core::graph::RouteGraph;

/// A barrier that will never complete, and the work it was holding.
#[derive(Debug, Clone)]
pub struct Abandoned {
    /// Which agent was waiting, in which chain.
    pub key: BarrierKey,
    /// Upstreams that never arrived and now never can.
    pub missing: Vec<AgentName>,
    /// Flights that were parked behind it.
    ///
    /// Returned rather than dropped so the Tower can say what was lost. Work that vanishes without
    /// a record is indistinguishable from work that was never asked for.
    pub stranded: usize,
}

impl std::fmt::Display for Abandoned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let missing = self
            .missing
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");

        write!(
            f,
            "`{}` will never wake: nothing live can still deliver {missing}",
            self.key.to
        )
    }
}

/// The barriers a running factory is holding.
#[derive(Debug, Default)]
pub struct Barriers {
    live: Mutex<BTreeMap<BarrierKey, Barrier>>,
}

impl Barriers {
    /// No barriers held.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Offers a flight to whichever barrier guards its destination.
    ///
    /// Returns `None` when the destination declares no join, which is the common case and means
    /// the flight should simply be run.
    ///
    /// # Panics
    ///
    /// Never; the lock is not held across a call out.
    pub fn deliver(&self, graph: &RouteGraph, flight: Flight) -> Option<Delivery> {
        let spec = graph.join_for(&flight.to)?;
        let key = BarrierKey::new(flight.itinerary.clone(), flight.to.clone());

        let mut live = self.live.lock().ok()?;
        let barrier = live.entry(key).or_insert_with(|| Barrier::from_spec(spec));

        Some(barrier.deliver(flight))
    }

    /// Gives up every barrier that no live run could still satisfy.
    ///
    /// `live_agents` is who is currently running or queued to run. A barrier is dead when none of
    /// its missing upstreams can be reached from any of them — at which point waiting is not
    /// patience, it is a hang.
    pub fn abandon_unreachable(
        &self,
        graph: &RouteGraph,
        live_agents: &BTreeSet<AgentName>,
    ) -> Vec<Abandoned> {
        let Ok(mut held) = self.live.lock() else {
            return Vec::new();
        };

        let mut given_up = Vec::new();

        held.retain(|key, barrier| {
            // A barrier that has already woken its agent is not waiting for anything; leaving it
            // in place is what lets the next wave reset it.
            if barrier.has_released() || barrier.is_reachable(graph, live_agents) {
                return true;
            }

            given_up.push(Abandoned {
                key: key.clone(),
                missing: barrier.waiting_for(),
                stranded: barrier.parked_count(),
            });

            false
        });

        given_up
    }

    /// Every barrier currently holding work, for reporting.
    #[must_use]
    pub fn waiting(&self) -> Vec<(BarrierKey, Vec<AgentName>)> {
        let Ok(held) = self.live.lock() else {
            return Vec::new();
        };

        held.iter()
            .filter(|(_, barrier)| !barrier.has_released() && barrier.parked_count() > 0)
            .map(|(key, barrier)| (key.clone(), barrier.waiting_for()))
            .collect()
    }

    /// How many barriers are held, for reporting.
    #[must_use]
    pub fn count(&self) -> usize {
        self.live.lock().map_or(0, |held| held.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layover_core::config::Config;
    use layover_core::flight::{ItineraryId, Origin};

    const FACTORY: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"

[runners.shell]
command = ["echo"]

[agents.developer]
prompt = "develop"
entry = true

[agents.tester]
prompt = "test"

[agents.reviewer]
prompt = "review"

[agents.publisher]
prompt = "publish"

[pipelines.build]
entry = "developer"

[[routes]]
from = "developer"
to = "tester"

[[routes]]
from = "developer"
to = "reviewer"

[[routes]]
from = ["tester", "reviewer"]
to = "publisher"
join = "all"
"#;

    fn graph() -> RouteGraph {
        let config: Config = toml::from_str(FACTORY).expect("the fixture factory parses");
        RouteGraph::from_config(&config)
    }

    fn flight(chain: &ItineraryId, from: &str, to: &str) -> Flight {
        Flight::new(
            chain.clone(),
            Origin::Agent(AgentName::new(from)),
            AgentName::new(to),
            "verdict",
            3,
        )
    }

    fn live(names: &[&str]) -> BTreeSet<AgentName> {
        names.iter().map(|n| AgentName::new(*n)).collect()
    }

    #[test]
    fn an_unjoined_destination_has_no_barrier_to_meet() {
        // The common case: most agents are not joined, and asking about a barrier that does not
        // exist must not create one.
        let barriers = Barriers::new();
        let chain = ItineraryId::generate();

        let outcome = barriers.deliver(&graph(), flight(&chain, "developer", "tester"));

        assert!(outcome.is_none(), "no join is declared on `tester`");
        assert_eq!(barriers.count(), 0, "nothing should have been created");
    }

    #[test]
    fn the_first_of_two_upstreams_is_parked_rather_than_run() {
        let barriers = Barriers::new();
        let chain = ItineraryId::generate();

        let outcome = barriers
            .deliver(&graph(), flight(&chain, "tester", "publisher"))
            .expect("publisher is joined");

        match outcome {
            Delivery::Parked { waiting_for } => {
                assert_eq!(waiting_for, [AgentName::new("reviewer")]);
            }
            other => panic!("expected a park, got {other:?}"),
        }
    }

    #[test]
    fn the_second_upstream_releases_the_agent_once_with_both_flights() {
        // Once, not twice. Two edges into one agent without a join fire it twice, which for a
        // publisher means two pull requests.
        let barriers = Barriers::new();
        let graph = graph();
        let chain = ItineraryId::generate();

        barriers.deliver(&graph, flight(&chain, "tester", "publisher"));
        let outcome = barriers
            .deliver(&graph, flight(&chain, "reviewer", "publisher"))
            .expect("publisher is joined");

        match outcome {
            Delivery::Ready(flights) => assert_eq!(flights.len(), 2, "both verdicts arrive"),
            other => panic!("expected a release, got {other:?}"),
        }
    }

    #[test]
    fn two_chains_waiting_on_the_same_agent_do_not_satisfy_each_other() {
        // A barrier is keyed by itinerary as well as agent. Sharing one would let a verdict about
        // one work item release the publisher for another.
        let barriers = Barriers::new();
        let graph = graph();
        let first = ItineraryId::generate();
        let second = ItineraryId::generate();

        barriers.deliver(&graph, flight(&first, "tester", "publisher"));
        let outcome = barriers
            .deliver(&graph, flight(&second, "reviewer", "publisher"))
            .expect("publisher is joined");

        assert!(
            matches!(outcome, Delivery::Parked { .. }),
            "a different chain must not complete this one: {outcome:?}"
        );
        assert_eq!(barriers.count(), 2, "one barrier per chain");
    }

    #[test]
    fn a_barrier_nothing_can_still_satisfy_is_given_up_and_says_what_was_lost() {
        // Waiting for an upstream no live run can produce is not patience, it is a hang — and a
        // silent one, which is the worst outcome in the system.
        let barriers = Barriers::new();
        let graph = graph();
        let chain = ItineraryId::generate();

        barriers.deliver(&graph, flight(&chain, "tester", "publisher"));

        // Nothing is running that could ever reach `reviewer`.
        let given_up = barriers.abandon_unreachable(&graph, &live(&["publisher"]));

        assert_eq!(given_up.len(), 1);
        assert_eq!(given_up[0].missing, [AgentName::new("reviewer")]);
        assert_eq!(
            given_up[0].stranded, 1,
            "the parked flight is accounted for"
        );
        assert!(
            given_up[0].to_string().contains("will never wake"),
            "{}",
            given_up[0]
        );
        assert_eq!(barriers.count(), 0, "a dead barrier is not kept");
    }

    #[test]
    fn a_barrier_whose_upstream_is_still_reachable_is_left_alone() {
        let barriers = Barriers::new();
        let graph = graph();
        let chain = ItineraryId::generate();

        barriers.deliver(&graph, flight(&chain, "tester", "publisher"));

        // `developer` is live and can still reach `reviewer`.
        let given_up = barriers.abandon_unreachable(&graph, &live(&["developer"]));

        assert!(
            given_up.is_empty(),
            "patience is correct here: {given_up:?}"
        );
        assert_eq!(barriers.count(), 1);
    }

    #[test]
    fn a_released_barrier_is_kept_so_the_next_wave_can_reset_it() {
        // A loop re-runs the same join. Discarding the barrier on release would lose the record
        // that it ever fired, and an `any` join would wake its agent once per straggler.
        let barriers = Barriers::new();
        let graph = graph();
        let chain = ItineraryId::generate();

        barriers.deliver(&graph, flight(&chain, "tester", "publisher"));
        barriers.deliver(&graph, flight(&chain, "reviewer", "publisher"));

        let given_up = barriers.abandon_unreachable(&graph, &live(&["publisher"]));

        assert!(given_up.is_empty(), "a released barrier waits for nothing");
        assert_eq!(barriers.count(), 1);
    }

    #[test]
    fn a_second_verdict_from_one_upstream_starts_a_fresh_wave() {
        // The failure loop: the developer fixes what the tester found and the tester reports
        // again. The reviewer's earlier approval must not count for the new code.
        let barriers = Barriers::new();
        let graph = graph();
        let chain = ItineraryId::generate();

        barriers.deliver(&graph, flight(&chain, "tester", "publisher"));
        barriers.deliver(&graph, flight(&chain, "reviewer", "publisher"));

        // Round two: the tester reports on the fix.
        let outcome = barriers
            .deliver(&graph, flight(&chain, "tester", "publisher"))
            .expect("publisher is joined");

        match outcome {
            Delivery::Parked { waiting_for } => {
                assert_eq!(
                    waiting_for,
                    [AgentName::new("reviewer")],
                    "the reviewer must look again at the new code"
                );
            }
            other => panic!("a stale approval must not release the publisher: {other:?}"),
        }
    }

    #[test]
    fn a_human_can_still_reach_an_agent_that_sits_behind_a_join() {
        // A join says which inputs an agent needs together, not when it is allowed to run.
        let barriers = Barriers::new();
        let chain = ItineraryId::generate();

        let direct = Flight::new(
            chain,
            Origin::Human,
            AgentName::new("publisher"),
            "publish it anyway",
            3,
        );

        let outcome = barriers
            .deliver(&graph(), direct)
            .expect("publisher is joined");

        assert!(
            matches!(outcome, Delivery::Direct(_)),
            "a human trigger bypasses the barrier: {outcome:?}"
        );
    }

    #[test]
    fn what_is_waiting_can_be_reported_without_disturbing_it() {
        let barriers = Barriers::new();
        let graph = graph();
        let chain = ItineraryId::generate();

        barriers.deliver(&graph, flight(&chain, "tester", "publisher"));

        let waiting = barriers.waiting();

        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].0.to, AgentName::new("publisher"));
        assert_eq!(waiting[0].1, [AgentName::new("reviewer")]);
    }
}
