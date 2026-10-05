//! Running the queue: several agents at once, up to `max_concurrent_runs`.
//!
//! # How a flight gets a slot
//!
//! One coordinator — the thread that called [`Factory::drain`] or [`Factory::dispatch`] — takes
//! flights off the queue in order, admits each against its chain's rails, starts its process, and
//! hands the running child to a worker thread of its own. The worker waits for it, prices it and
//! writes it down, then says so over a channel. The coordinator starts the next flight as soon as
//! a slot is free.
//!
//! Everything that decides *whether* work may start stays on the coordinator: unqueueing,
//! barriers, admission. That keeps the guarantees the one-at-a-time loop gave without a lock
//! around them — a flight is taken off the queue before it runs and never taken twice, a barrier
//! sees its arrivals one at a time, and a chain's run cap is charged in the order its flights were
//! admitted. Only waiting, which is where all the time goes, happens in parallel.
//!
//! # Order, and agents that must not overlap
//!
//! First in, first out, among the flights that can start. A flight whose agent already has as
//! many runs alive as its own `max_concurrent` allows waits where it is, and the flights behind it
//! for other agents go ahead. Waiting for the one agent is what the cap asks for; holding the whole
//! factory behind it is not.
//!
//! # Why excess work waits rather than being refused
//!
//! Every money rail refuses, because money spent is gone. This one protects a machine, and a
//! machine that is busy now will not be busy in a minute. A sweep that finds twelve pull requests
//! reviews twelve, four at a time — not four, with eight silently dropped.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::sync::PoisonError;
use std::sync::mpsc::{self, Sender};
use std::thread::Scope;
use std::time::Duration;

use jiff::Timestamp;
use layover_core::agent::AgentName;
use layover_core::flight::{Flight, ItineraryId, RunId};
use layover_core::itinerary::Itinerary;
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;

use crate::cost::Reported;
use crate::dispatch::Authorised;
use crate::factory::{Dispatched, Drained, Factory};
use crate::spawn::Started;

/// How often a busy coordinator looks again, for a Ground Stop and — when it runs the queue — for
/// newly queued work, between runs finishing.
const POLL: Duration = Duration::from_millis(250);

/// A run the factory has started and not yet seen finish.
#[derive(Debug, Clone)]
pub(crate) struct InFlight {
    /// Which agent it is.
    pub agent: AgentName,
    /// The pipeline that chain belongs to, when one does.
    pub pipeline: Option<PipelineName>,
}

/// What a run is, once admitted and started: everything its settling needs.
pub(crate) struct Ticket<'a> {
    pub run: RunId,
    pub chain: ItineraryId,
    pub authorised: Authorised<'a>,
    pub started_at: Timestamp,
    pub queued_at: Option<Timestamp>,
    pub sent_by: Option<Vec<AgentName>>,
}

/// A run whose process is alive.
pub(crate) struct Launched<'a> {
    pub ticket: Ticket<'a>,
    pub started: Started,
    pub token: Option<String>,
}

/// A run that finished, on its way back to the coordinator.
struct Landed {
    /// The flight as it was queued, which is what `refill` is told about.
    queued: Flight,
    /// The flight that ran — for a released join, the one carrying every arrival.
    flight: Flight,
    result: Dispatched,
}

/// When the coordinator asks for more work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refill {
    /// After a run finishes, and when nothing is running. Only a run can send a flight, so this
    /// misses nothing a drain was handed.
    AfterRuns,
    /// Also every [`POLL`] while runs are alive, because a person or a schedule can queue work at
    /// any moment and it should start as soon as there is a slot, not when a run happens to end.
    Continuously,
}

/// The queue as the coordinator sees it: waiting flights in order, and every flight already taken.
struct Waiting {
    waiting: VecDeque<Queued>,
    taken: HashSet<String>,
}

impl Waiting {
    fn new(pending: Vec<Queued>) -> Self {
        let mut queue = Self {
            waiting: VecDeque::new(),
            taken: HashSet::new(),
        };
        queue.merge(pending);
        queue
    }

    /// Adds what is not already waiting and was never taken. A flight taken once is never taken
    /// again, even if a queue that failed to forget it offers it back.
    fn merge(&mut self, fresh: Vec<Queued>) {
        for queued in fresh {
            let id = queued.flight.id.as_str();
            if self.taken.contains(id)
                || self
                    .waiting
                    .iter()
                    .any(|waiting| waiting.flight.id.as_str() == id)
            {
                continue;
            }
            self.waiting.push_back(queued);
        }
    }

    fn take(&mut self, index: usize) -> Option<Queued> {
        let queued = self.waiting.remove(index)?;
        self.taken.insert(queued.flight.id.as_str().to_owned());
        Some(queued)
    }
}

/// Charges a chain for what one of its runs cost.
///
/// Debited even when the run failed. Money spent is money spent, and a chain that could retry
/// forever on failures without paying for them is not bounded. Dollars a runner printed and Copilot
/// credits priced at a published rate are both measurements; a guess or a hole is not, and debits
/// nothing. That a run measured nothing is kept where it is read — its record's cost source, which
/// turns every total it is part of into a floor — rather than in a count on the chain nothing reads.
pub(crate) fn charge(itinerary: &mut Itinerary, reported: &Reported) {
    if reported.source.is_measured() {
        itinerary.debit_fuel(reported.usd);
    }
}

impl Factory {
    /// How many runs this factory has alive now.
    #[must_use]
    pub fn alive_runs(&self) -> usize {
        self.inflight.lock().map_or(0, |inflight| inflight.len())
    }

    /// How many runs of `agent` are alive now.
    #[must_use]
    pub fn alive_of(&self, agent: &AgentName) -> usize {
        self.inflight.lock().map_or(0, |inflight| {
            inflight.values().filter(|run| run.agent == *agent).count()
        })
    }

    /// The pipelines that have a run alive now.
    ///
    /// A schedule's previous wave is still going while any of its runs is — including a spawned
    /// chain's, which belongs to the pipeline that spawned it — and the queue alone cannot say so,
    /// because a flight leaves the queue the moment it starts.
    #[must_use]
    pub fn busy_pipelines(&self) -> BTreeSet<PipelineName> {
        self.inflight.lock().map_or_else(
            |_| BTreeSet::new(),
            |inflight| {
                inflight
                    .values()
                    .filter_map(|run| run.pipeline.clone())
                    .collect()
            },
        )
    }

    /// The agents with a run alive now, which is what "nothing live could still deliver" asks.
    pub(crate) fn alive_agents(&self) -> BTreeSet<AgentName> {
        self.inflight.lock().map_or_else(
            |_| BTreeSet::new(),
            |inflight| inflight.values().map(|run| run.agent.clone()).collect(),
        )
    }

    /// Notes that `run` is alive.
    pub(crate) fn hold(&self, run: RunId, inflight: InFlight) {
        self.inflight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(run, inflight);
    }

    /// Notes that `run` is no longer alive.
    pub(crate) fn forget(&self, run: &RunId) {
        self.inflight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(run);
    }

    /// Whether `agent` already has as many runs alive as its own `max_concurrent` allows.
    fn agent_is_full(&self, agent: &AgentName) -> bool {
        self.config
            .agents
            .get(agent)
            .and_then(|definition| definition.max_concurrent)
            .is_some_and(|cap| self.alive_of(agent) >= cap)
    }

    /// Runs every flight waiting in `pending`, and every flight those runs send, until nothing is
    /// left — up to `max_concurrent_runs` at a time.
    ///
    /// Each is taken off the queue **before** it runs. A flight that crashes the factory mid-run
    /// must not come back on restart and run again: an agent that opened a pull request and was
    /// interrupted before its outcome was recorded would open a second one.
    ///
    /// The loop is bounded by the rails rather than by a count: each flight spends a Hop from a
    /// shared itinerary and each run spends Fuel and a slot against the run cap, so a chain that
    /// will not settle is cut by the same mechanism that bounds every other chain. It also stops
    /// once nothing is running and asking for more started nothing: only a run can send a flight,
    /// so a round in which every flight was refused cannot have produced new work.
    pub fn drain(
        &self,
        pending: Vec<Queued>,
        mut unqueue: impl FnMut(&Flight),
        mut report: impl FnMut(&Flight, &Dispatched),
    ) -> Drained {
        self.drain_with(pending, &mut unqueue, &mut report, |_| Vec::new())
    }

    /// Drains, asking `refill` for newly queued work after each run finishes.
    ///
    /// `refill` is told which flights were dealt with since it was last asked.
    pub fn drain_with(
        &self,
        pending: Vec<Queued>,
        unqueue: &mut impl FnMut(&Flight),
        report: &mut impl FnMut(&Flight, &Dispatched),
        refill: impl FnMut(&[Flight]) -> Vec<Queued>,
    ) -> Drained {
        self.coordinate(
            pending,
            &mut |flight| {
                unqueue(flight);
                true
            },
            report,
            refill,
            &|| false,
            Refill::AfterRuns,
        )
    }

    /// Runs the queue as a Tower does: like [`Factory::drain_with`], but also looking for newly
    /// queued work while runs are alive, so work queued by a person or a schedule starts as soon as
    /// there is a slot for it.
    ///
    /// `unqueue` says whether the flight was still queued; one that was not — cancelled a moment
    /// ago — is not run. Once `stop` says so, nothing new starts, and this returns when what is
    /// running has finished.
    pub fn dispatch(
        &self,
        pending: Vec<Queued>,
        unqueue: &mut impl FnMut(&Flight) -> bool,
        report: &mut impl FnMut(&Flight, &Dispatched),
        refill: impl FnMut(&[Flight]) -> Vec<Queued>,
        stop: &dyn Fn() -> bool,
    ) -> Drained {
        self.coordinate(pending, unqueue, report, refill, stop, Refill::Continuously)
    }

    fn coordinate(
        &self,
        pending: Vec<Queued>,
        unqueue: &mut dyn FnMut(&Flight) -> bool,
        report: &mut dyn FnMut(&Flight, &Dispatched),
        mut refill: impl FnMut(&[Flight]) -> Vec<Queued>,
        stop: &dyn Fn() -> bool,
        when: Refill,
    ) -> Drained {
        let (tx, rx) = mpsc::channel::<Landed>();
        let mut queue = Waiting::new(pending);
        let mut done: Vec<Flight> = Vec::new();
        let mut ran = 0;
        let mut flying = 0_usize;
        let mut halted = false;
        // Set when nothing is running and asking for more work started nothing.
        let mut dry = false;

        std::thread::scope(|scope| {
            loop {
                let grounded = self.ground_stop_engaged();
                halted |= grounded;
                let stopping = grounded || stop();

                if !stopping {
                    let started =
                        self.start_what_fits(scope, &tx, &mut queue, &mut done, unqueue, report);
                    flying += started;
                    if started > 0 {
                        dry = false;
                    }
                }

                if flying == 0 {
                    if stopping || dry {
                        break;
                    }
                    queue.merge(refill(&done));
                    done.clear();
                    if queue.waiting.is_empty() {
                        break;
                    }
                    dry = true;
                    continue;
                }

                let mut landed = Vec::new();
                if let Ok(first) = rx.recv_timeout(POLL) {
                    landed.push(first);
                    landed.extend(rx.try_iter());
                }
                for arrival in landed.drain(..) {
                    flying -= 1;
                    if matches!(arrival.result, Dispatched::Ran { .. }) {
                        ran += 1;
                    }
                    report(&arrival.flight, &arrival.result);
                    done.push(arrival.queued);
                }

                let ask = match when {
                    Refill::AfterRuns => !done.is_empty(),
                    Refill::Continuously => true,
                };
                if ask && !stopping {
                    queue.merge(refill(&done));
                    done.clear();
                }
            }
        });

        // Nothing this drain started is running, so any barrier still holding work is waiting for
        // something that will never arrive — unless an agent could still deliver it: one alive now,
        // under another drain, or one a flight still waiting here is for, when this stopped early
        // or every slot was taken. Giving up loudly beats a silent permanent stall: a failure at
        // least says something happened.
        let abandoned = if halted {
            Vec::new()
        } else {
            let mut could_deliver = self.alive_agents();
            could_deliver.extend(queue.waiting.iter().map(|queued| queued.flight.to.clone()));
            self.barriers.abandon_unreachable(
                &self.routes,
                |id| self.chains.scope_of(id),
                &could_deliver,
            )
        };

        Drained { ran, abandoned }
    }

    /// Starts every waiting flight that fits, in order, and says how many runs it started.
    fn start_what_fits<'scope, 'env>(
        &'env self,
        scope: &'scope Scope<'scope, 'env>,
        tx: &Sender<Landed>,
        queue: &mut Waiting,
        done: &mut Vec<Flight>,
        unqueue: &mut dyn FnMut(&Flight) -> bool,
        report: &mut dyn FnMut(&Flight, &Dispatched),
    ) -> usize {
        let limit = self.config.defaults.max_concurrent_runs.max(1);
        let mut started = 0;
        let mut index = 0;

        while index < queue.waiting.len() && self.alive_runs() < limit {
            if self.agent_is_full(&queue.waiting[index].flight.to) {
                index += 1;
                continue;
            }
            let Some(queued) = queue.take(index) else {
                break;
            };

            // Off the queue before it runs. One that was no longer there was cancelled, or taken by
            // something else, and running it anyway is how work gets done twice.
            if !unqueue(&queued.flight) {
                done.push(queued.flight);
                continue;
            }

            match self.launch_queued(&queued, report) {
                Some((flight, launched)) => {
                    let tx = tx.clone();
                    scope.spawn(move || {
                        let chain = launched.ticket.chain.clone();
                        let result = self.finish(launched, |reported| {
                            let _ = self
                                .chains
                                .with(&chain, &self.config.defaults, |itinerary| {
                                    charge(itinerary, reported);
                                });
                        });
                        let _ = tx.send(Landed {
                            queued: queued.flight,
                            flight,
                            result,
                        });
                    });
                    started += 1;
                }
                None => done.push(queued.flight),
            }
        }

        started
    }

    /// Takes one queued flight as far as a running process, reporting it if it stops short.
    fn launch_queued<'a>(
        &'a self,
        queued: &Queued,
        report: &mut dyn FnMut(&Flight, &Dispatched),
    ) -> Option<(Flight, Launched<'a>)> {
        // Recorded before anything runs, because this is where a chain's pipeline and flags
        // enter the process: from the trigger for a fresh chain, from the queued flight for one
        // resumed after a restart. Every run after this one reads them from here.
        self.chains.opened_by(
            &queued.flight.itinerary,
            queued.pipeline.as_ref(),
            &queued.flags,
            &queued.within,
        );
        self.chains
            .continuing(&queued.flight.itinerary, queued.continues.as_ref());

        // A restarted run of a released join goes straight to its agent. The barrier it passed
        // went with the Tower that held it, and a fresh one would wait for upstreams that finished
        // long ago. Which arrivals it carried went with that Tower too, so they are not recorded
        // rather than reduced to the first.
        let (flight, joined, sent_by) = if queued.released {
            (queued.flight.clone(), true, None)
        } else {
            let mut parked = |flight: &Flight, result: &Dispatched| report(flight, result);
            let passed = self.past_the_barrier(&queued.flight, &mut parked)?;
            (passed.flight, passed.joined, Some(passed.sent_by))
        };

        // Waiting at a barrier is the design; only the wait after it releases says anything
        // about dispatch, so a released join counts as queued at the moment it released.
        let queued_at = if joined {
            Timestamp::now()
        } else {
            flight.sent_at
        };

        // The chain is held only while its rails are charged. Every flight in one causal chain is
        // accounted against the same Hops, Fuel and run cap, and runs of it that are alive at once
        // must each see what the last one admitted left.
        let sender = flight.from.agent().cloned();
        let admitted = self
            .chains
            .with(&flight.itinerary, &self.config.defaults, |chain| {
                self.admit(chain, sender.as_ref(), &flight)
            })
            .unwrap_or_else(|| {
                Err(Dispatched::Failed(
                    "the itinerary ledger was poisoned".to_owned(),
                ))
            });
        let authorised = match admitted {
            Ok(authorised) => authorised,
            Err(result) => {
                report(&flight, &result);
                return None;
            }
        };

        // What the live record keeps is what a restart would run again: for a released join, the
        // one flight carrying every arrival, marked as already past its barrier.
        let kept = if joined {
            Queued {
                flight: flight.clone(),
                ..queued.clone()
            }
            .already_released()
        } else {
            queued.clone()
        };

        let origin = (!joined).then_some(&flight.from);
        let start = crate::factory::Start {
            origin,
            queued: Some(&kept),
            queued_at: Some(queued_at),
            sent_by,
        };
        match self.launch(&flight.itinerary, authorised, &flight, start) {
            Ok(launched) => Some((flight, launched)),
            Err(result) => {
                report(&flight, &result);
                None
            }
        }
    }
}

/// Runs alive, by run.
pub(crate) type Registry = BTreeMap<RunId, InFlight>;
