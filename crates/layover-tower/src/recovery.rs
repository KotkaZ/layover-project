//! Settling the runs a previous Tower started and never saw finish.
//!
//! A Tower that goes away — a restart, a crash, a closed terminal — leaves a live record behind for
//! every run it was watching, and nothing else: no exit code arrived, nothing was priced, nothing
//! reached history. The next Tower to open the factory settles each one before it starts anything.
//!
//! # A run still alive is stopped
//!
//! Its process may well outlive the Tower — on Windows a child routinely does — but it is cut off.
//! Its Layover endpoint and token died with the Tower that minted them, so nothing it sends,
//! reports or books can arrive, and left alone it keeps spending on work nobody will receive.
//! Stopping it is also what makes it *confirmed* gone, which recovery requires before it starts a
//! replacement: a second run beside a first that never stopped does the work twice.
//!
//! # Then the ordinary rules decide
//!
//! Whether the work starts again is the agent's `recovery` policy and `max_recovery_attempts`, as
//! for any interruption, and a Ground Stop forbids it. The restart is told what happened and warned
//! to check before repeating anything the first run may already have done.
//!
//! # Order
//!
//! Written to history, then the live record removed, then the restart queued. A crash between the
//! last two loses the restart and leaves an `interrupted` run in history, which somebody can see;
//! the other order would, after a second crash, restart the same work twice.

use std::collections::BTreeSet;
use std::fmt;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use layover_core::agent::AgentName;
use layover_core::cost::TokenUsage;
use layover_core::cost::Window;
use layover_core::flight::{Flight, ItineraryId, RunId};
use layover_core::handover::{ChildState, Interruption, Recovery, authorize_recovery};
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_core::run::{Outcome, RunRecord};

use crate::factory::Factory;
use crate::owner::Owner;
use crate::probe::probe;
use crate::state::{Live, Verdict};

/// How long a stopped run is given to disappear before it counts as unaccounted for.
const STOPPING: Duration = Duration::from_secs(10);

/// What became of the process a live record named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// It was still running, and was stopped.
    Stopped,
    /// It had already exited.
    Exited,
    /// It had exited, and its identifier now belongs to another program, which is left alone.
    Reused,
    /// Whether it is still running could not be established: the check failed, or it survived
    /// being stopped. Nothing is started beside it.
    Unknown,
}

impl Found {
    const fn child(self) -> ChildState {
        match self {
            Self::Unknown => ChildState::Unknown,
            Self::Stopped | Self::Exited | Self::Reused => ChildState::Gone,
        }
    }
}

impl fmt::Display for Found {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stopped => "it was still running, cut off from Layover, and was stopped",
            Self::Exited => "it had already exited",
            Self::Reused => {
                "it had exited, and its process identifier now belongs to another program"
            }
            Self::Unknown => "whether it is still running could not be established",
        })
    }
}

/// A run a previous Tower left behind, and what was done about it.
#[derive(Debug, Clone)]
pub struct Settled {
    /// The run.
    pub run: RunId,
    /// Its agent.
    pub agent: AgentName,
    /// What became of its process.
    pub found: Found,
    /// The attempt its work was queued again as, or why it was not.
    pub restarted: Result<u32, String>,
}

impl fmt::Display for Settled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` ({}) was interrupted by a restart: {}; ",
            self.agent, self.run, self.found
        )?;
        match &self.restarted {
            Ok(attempt) => write!(f, "restarted as attempt {attempt}"),
            Err(why) => write!(f, "not restarted: {why}"),
        }
    }
}

impl Factory {
    /// Settles every run recorded as live whose Tower has gone: stops it if it is still running,
    /// prices and records it as interrupted, and — where its agent's `recovery` policy allows —
    /// hands `queue` the work to start again.
    ///
    /// Call it before starting anything. A run another living Tower is watching is left alone.
    pub fn reconcile(&self, queue: &mut dyn FnMut(Queued) -> Result<(), String>) -> Vec<Settled> {
        let root = self.live.root().to_path_buf();
        let Ok(recorded) = self.live.live() else {
            return Vec::new();
        };

        let mut settled = Vec::new();
        let mut gone = BTreeSet::new();
        for live in recorded {
            if let Some(owner) = &live.owner {
                if Owner::is_alive(&root, owner) {
                    continue;
                }
                gone.insert(owner.clone());
            }
            settled.push(self.settle_left_behind(&live, queue));
        }

        for owner in gone {
            Owner::clear(&root, &owner);
        }
        settled
    }

    fn settle_left_behind(
        &self,
        live: &Live,
        queue: &mut dyn FnMut(Queued) -> Result<(), String>,
    ) -> Settled {
        let found = make_sure_gone(live);
        let restart = self.restart_for(live, found);

        let transcript = std::fs::read_to_string(live.hangar.join(crate::spawn::TRANSCRIPT_FILE))
            .unwrap_or_default();
        let reported =
            crate::cost::from_transcript(&transcript, self.config.copilot.usd_per_credit);

        // Charged to the chain as any run is, so a restart is admitted against what the run it
        // replaces spent and counts as one more run of the chain, not as its first.
        let _ = self
            .chains
            .with(&live.itinerary, &self.config.defaults, |chain| {
                let _ = chain.record_run_started();
                crate::dispatcher::charge(chain, &reported);
            });

        let detail = match (&live.queued, &restart) {
            // Said plainly: a person reading the Runs list should know this is not a run of the
            // workflow that failed, but one that was lost, and what to do about it.
            (None, _) => format!(
                "lost when the Tower stopped (process {}; {found}). Layover 1.3.0 and earlier did not \
                 record its work, so it could not be restarted. Re-trigger it if it is still needed.",
                live.pid
            ),
            (Some(_), Ok(queued)) => format!(
                "the Tower went away while it was running (process {}); {found}; restarting it as \
                 attempt {}",
                live.pid,
                queued.attempt()
            ),
            (Some(_), Err(why)) => format!(
                "the Tower went away while it was running (process {}); {found}; not restarted: {why}",
                live.pid
            ),
        };
        let agent = self.config.agents.get(&live.agent);
        let pipeline = live
            .queued
            .as_ref()
            .and_then(|queued| queued.pipeline.clone())
            .or_else(|| self.pipeline_on_record(&live.itinerary));
        self.record(&RunRecord {
            run: live.run.clone(),
            itinerary: live.itinerary.clone(),
            agent: live.agent.clone(),
            pipeline,
            model: agent.and_then(|agent| agent.model.clone()),
            outcome: Outcome::Interrupted,
            queued_at: None,
            started_at: live.started_at,
            // When it was last seen alive, not when this Tower got round to noticing: a run that
            // died twenty-nine minutes in must not read as having taken the six hours the Tower
            // was down. Only a run stopped just now ended now.
            finished_at: Some(match found {
                Found::Stopped => Timestamp::now(),
                Found::Exited | Found::Reused | Found::Unknown => last_seen_alive(live),
            }),
            usd: reported.usd,
            source: reported.source,
            usage: TokenUsage {
                input: reported.input_tokens,
                output: reported.output_tokens,
                ..TokenUsage::default()
            },
            exit_code: None,
            detail: Some(detail),
            blocked_on: None,
            pid: None,
            flags: live
                .queued
                .as_ref()
                .filter(|queued| queued.pipeline.is_some())
                .map(|queued| queued.flags.clone())
                .unwrap_or_default(),
            continues: live
                .queued
                .as_ref()
                .and_then(|queued| queued.continues.clone()),
        });
        self.age_learnings(&live.agent);
        let _ = self.live.finished(&live.run);

        let restarted = restart.and_then(|queued| {
            let attempt = queued.attempt();
            queue(queued)
                .map(|()| attempt)
                .map_err(|error| format!("the restart could not be queued: {error}"))
        });

        Settled {
            run: live.run.clone(),
            agent: live.agent.clone(),
            found,
            restarted,
        }
    }

    /// The work to start again, or why it may not be.
    fn restart_for(&self, live: &Live, found: Found) -> Result<Queued, String> {
        let queued = live.queued.as_ref().ok_or_else(|| {
            "its record was written by a release that did not keep the work it was given".to_owned()
        })?;
        let agent = self
            .config
            .agents
            .get(&live.agent)
            .ok_or_else(|| format!("`{}` is no longer declared", live.agent))?;

        // Said as itself rather than as "not retryable": a factory that restarts through its own
        // kill switch is not one anybody can stop, and whoever reads this should know that is why.
        if self.ground_stop_engaged() {
            return Err("a Ground Stop is engaged".to_owned());
        }

        let interruption = Interruption::TowerRestart;
        let attempt = queued.attempt();
        authorize_recovery(
            agent.recovery,
            &interruption,
            found.child(),
            attempt.saturating_sub(1),
            self.config.defaults.max_recovery_attempts,
        )
        .map_err(|denied| denied.to_string())?;

        // A new flight for the same work, in the same chain, from the same sender, so the route
        // check and the payload say what they said the first time. The chain, its pipeline, its
        // flags and its reach all come with it.
        let original = &queued.flight;
        let flight = Flight::new(
            original.itinerary.clone(),
            original.from.clone(),
            original.to.clone(),
            original.body.clone(),
            original.hops_remaining,
        );

        Ok(Queued {
            flight,
            ..queued.clone()
        }
        .restarting(Recovery {
            previous: live.run.clone(),
            interruption,
            attempt: attempt.saturating_add(1),
        }))
    }
}

impl Factory {
    /// The workflow a chain belonged to, as anything still on disk records it: another of its runs
    /// in history, work of it still queued, work it set down, or a help request it raised.
    ///
    /// For a run whose own record did not say, which every run started by 1.3.0 and earlier is.
    /// `None` only when nothing records it — better than a guess, which would file the run under a
    /// workflow it never belonged to.
    fn pipeline_on_record(&self, chain: &ItineraryId) -> Option<PipelineName> {
        let everything = Window::AllTime.resolve(
            &(Timestamp::now() + jiff::SignedDuration::from_mins(1))
                .to_zoned(jiff::tz::TimeZone::UTC),
        );
        let history = layover_store::History::open(self.history_dir()).ok();
        let from_history = || {
            history?
                .runs(&everything, &layover_store::RunFilter::default())
                .ok()?
                .into_iter()
                .filter(|record| record.itinerary == *chain)
                .find_map(|record| record.pipeline)
        };
        let from_queue = || {
            self.journal
                .pending()
                .ok()?
                .into_iter()
                .filter(|queued| queued.flight.itinerary == *chain)
                .find_map(|queued| queued.pipeline)
        };
        let from_layovers = || {
            self.journal
                .layovers()
                .ok()?
                .into_iter()
                .filter(|layover| layover.booked_by == *chain)
                .find_map(|layover| layover.scope.and_then(|scope| scope.pipeline))
        };
        let from_help = || {
            self.journal
                .help(&everything, &layover_store::HelpFilter::default())
                .ok()?
                .into_iter()
                .filter(|request| request.itinerary == *chain)
                .find_map(|request| request.scope.and_then(|scope| scope.pipeline))
        };

        self.chains
            .pipeline_of(chain)
            .or_else(from_history)
            .or_else(from_queue)
            .or_else(from_layovers)
            .or_else(from_help)
    }
}

/// The last moment `live` is known to have been alive: the last write to its transcript, which
/// the CLI appends to as long as it runs, or its start when there is not one.
fn last_seen_alive(live: &Live) -> Timestamp {
    std::fs::metadata(live.hangar.join(crate::spawn::TRANSCRIPT_FILE))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| Timestamp::try_from(modified).ok())
        .filter(|written| *written >= live.started_at)
        .unwrap_or(live.started_at)
}

/// Establishes that the run's process is gone, stopping it if it is not.
fn make_sure_gone(live: &Live) -> Found {
    match probe(live.pid, live.started_at) {
        Some(Verdict::ConfirmedGone) => Found::Exited,
        Some(Verdict::Reused) => Found::Reused,
        None => Found::Unknown,
        Some(Verdict::StillRunning) => {
            let _ = crate::wait::kill_tree(live.pid);
            let deadline = Instant::now() + STOPPING;
            loop {
                match probe(live.pid, live.started_at) {
                    Some(Verdict::ConfirmedGone | Verdict::Reused) => return Found::Stopped,
                    Some(Verdict::StillRunning) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    _ => return Found::Unknown,
                }
            }
        }
    }
}
