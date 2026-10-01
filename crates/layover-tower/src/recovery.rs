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
use layover_core::flight::{Flight, RunId};
use layover_core::handover::{ChildState, Interruption, Recovery, authorize_recovery};
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

        let decided = match &restart {
            Ok(queued) => format!("restarting it as attempt {}", queued.attempt()),
            Err(why) => format!("not restarted: {why}"),
        };
        let agent = self.config.agents.get(&live.agent);
        self.record(&RunRecord {
            run: live.run.clone(),
            itinerary: live.itinerary.clone(),
            agent: live.agent.clone(),
            pipeline: live
                .queued
                .as_ref()
                .and_then(|queued| queued.pipeline.clone()),
            model: agent.and_then(|agent| agent.model.clone()),
            outcome: Outcome::Interrupted,
            queued_at: None,
            started_at: live.started_at,
            finished_at: Some(Timestamp::now()),
            usd: reported.usd,
            source: reported.source,
            usage: TokenUsage {
                input: reported.input_tokens,
                output: reported.output_tokens,
                ..TokenUsage::default()
            },
            exit_code: None,
            detail: Some(format!(
                "the Tower went away while it was running (process {}); {found}; {decided}",
                live.pid
            )),
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
