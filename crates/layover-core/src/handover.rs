//! Starting a run that carries what an earlier one knew.
//!
//! Two problems turn out to be the same problem. A run dies when the machine restarts, and a human
//! watching a run go the wrong way wants to redirect it. Both were tempting to solve by keeping a
//! process alive — resuming a conversation, or piping input into a live child — and both are
//! solved here instead by **starting a new run and handing it what the old one had**.
//!
//! That choice keeps the most opinionated decision in the project intact: every run is still a
//! clean slate *process*. Nothing is resumed, no session is held open, and resident agents stay
//! out of scope along with the reentrancy hazard they bring. What changes is only how much context
//! a new run opens with.
//!
//! # The rails still apply
//!
//! A recovered or steered run is an ordinary run: it spends a hop, debits Fuel, counts against the
//! run cap and draws on the Reserve. Recovery additionally has [`RecoveryPolicy`] and an attempt
//! limit, because a crash loop that restarts itself forever is a fork bomb that looks like
//! resilience.
//!
//! # What this module does not decide
//!
//! [`Handover::brief`] renders a block of text *about* the previous run. How that block combines
//! with the agent's own prompt, the incoming flight body and `memory.md` is run-bootstrap
//! question 1 in `docs/roadmap.md`, and is still open. This produces the block; something else
//! decides where it goes.

use std::fmt;
use std::fmt::Write as _;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::flight::{Flight, RunId};

/// Why a run stopped without finishing.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Interruption {
    /// The Tower went away — a restart, a crash, a reboot — while the run was live.
    TowerRestart,
    /// The run exceeded `timeout_sec`.
    Timeout,
    /// The process exited non-zero.
    Crashed {
        /// Exit code, when the operating system reported one.
        exit_code: Option<i32>,
    },
    /// A Ground Stop halted it.
    GroundStop,
}

impl Interruption {
    /// Returns `true` when restarting the work could plausibly succeed.
    ///
    /// A Ground Stop is excluded deliberately: somebody pulled the handle, and a factory that
    /// restarts through its own kill switch is not one anybody can stop.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        !matches!(self, Self::GroundStop)
    }
}

impl fmt::Display for Interruption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TowerRestart => f.write_str("the Tower restarted while it was running"),
            Self::Timeout => f.write_str("it ran past its timeout"),
            Self::Crashed {
                exit_code: Some(code),
            } => write!(f, "it exited with code {code}"),
            Self::Crashed { exit_code: None } => f.write_str("it exited abnormally"),
            Self::GroundStop => f.write_str("a Ground Stop halted it"),
        }
    }
}

/// Whether an interrupted agent may be restarted without asking.
///
/// The question this answers is not "can we?" but "is it safe to do the work twice?". An agent
/// that reads and reports is harmless to re-run. One that opened a pull request is not, and
/// re-running it would open a second.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryPolicy {
    /// Restart automatically, up to the attempt limit.
    #[default]
    Automatic,
    /// Record the interruption and wait for a human to ask.
    Manual,
    /// Never restart. The itinerary is interrupted and stays that way.
    Never,
}

impl RecoveryPolicy {
    /// Returns `true` when the Tower may restart this agent on its own.
    #[must_use]
    pub fn is_automatic(&self) -> bool {
        matches!(self, Self::Automatic)
    }
}

/// A restart of work that was interrupted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    /// The run that did not finish.
    pub previous: RunId,
    /// What happened to it.
    pub interruption: Interruption,
    /// Which attempt this is. The first restart is attempt 2.
    pub attempt: u32,
}

/// A human redirecting work that is already under way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Steer {
    /// The run being redirected.
    pub previous: RunId,
    /// What the human said to do differently.
    pub note: String,
    /// When they said it.
    pub at: SystemTime,
}

/// Why a run is being started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// A flight arrived in the ordinary way.
    Dispatch,
    /// An earlier run was interrupted and the work is being restarted.
    Recovered(Recovery),
    /// A human redirected an earlier run.
    Steered(Steer),
}

impl Cause {
    /// Returns `true` when this run is repeating work an earlier one may have partly done.
    ///
    /// The distinction matters to an agent: work already done may need checking before it is done
    /// again, and a side effect already applied must not be applied twice.
    #[must_use]
    pub fn repeats_earlier_work(&self) -> bool {
        !matches!(self, Self::Dispatch)
    }
}

/// Everything a new run is told about the run it is taking over from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handover {
    /// Why this run is starting.
    pub cause: Cause,
    /// The flights the earlier run was given, which this one receives again.
    pub flights: Vec<Flight>,
    /// What the earlier run had recorded before it stopped, newest last.
    ///
    /// Whatever the store can honestly supply: lines the agent wrote to its memory, a summary of
    /// its transcript. Never a promise that the list is complete — an interrupted run stops
    /// mid-sentence by definition.
    pub progress: Vec<String>,
}

impl Handover {
    /// An ordinary first run, taking over from nothing.
    #[must_use]
    pub fn dispatch(flights: Vec<Flight>) -> Self {
        Self {
            cause: Cause::Dispatch,
            flights,
            progress: Vec::new(),
        }
    }

    /// A restart of interrupted work.
    #[must_use]
    pub fn recovered(recovery: Recovery, flights: Vec<Flight>) -> Self {
        Self {
            cause: Cause::Recovered(recovery),
            flights,
            progress: Vec::new(),
        }
    }

    /// A redirection of work already under way.
    #[must_use]
    pub fn steered(steer: Steer, flights: Vec<Flight>) -> Self {
        Self {
            cause: Cause::Steered(steer),
            flights,
            progress: Vec::new(),
        }
    }

    /// Adds what the earlier run had managed to record.
    #[must_use]
    pub fn with_progress(mut self, progress: Vec<String>) -> Self {
        self.progress = progress;
        self
    }

    /// Renders the block telling this run what it is taking over.
    ///
    /// Empty for an ordinary dispatch: a first run is taking over nothing, and a paragraph
    /// explaining that would be noise in every prompt in the factory.
    #[must_use]
    pub fn brief(&self) -> String {
        let mut out = String::new();

        match &self.cause {
            Cause::Dispatch => return out,
            Cause::Recovered(recovery) => {
                out.push_str("## You are continuing interrupted work\n\n");
                let _ = writeln!(
                    out,
                    "A previous run ({}) started this work and did not finish: {}. This is \
                     attempt {}.\n",
                    recovery.previous, recovery.interruption, recovery.attempt
                );
                out.push_str(
                    "You are a new process and remember none of it. Before repeating anything \
                     that changes the world — a commit, a comment, a published pull request — \
                     check whether the earlier run already did it. Doing it twice is worse than \
                     doing it late.\n\n",
                );
            }
            Cause::Steered(steer) => {
                out.push_str("## A human has redirected this work\n\n");
                let _ = writeln!(
                    out,
                    "A previous run ({}) was working on this. Their instruction takes precedence \
                     over the original request where the two disagree:\n",
                    steer.previous
                );
                let _ = writeln!(out, "> {}\n", steer.note.trim());
            }
        }

        if self.progress.is_empty() {
            out.push_str(
                "Nothing was recorded about what the earlier run had done, so assume it may have \
                 got anywhere from nowhere to almost finished.\n",
            );
        } else {
            out.push_str("What the earlier run recorded, oldest first:\n\n");
            for note in &self.progress {
                let _ = writeln!(out, "- {}", note.trim());
            }
            out.push_str(
                "\nThat list is what it managed to write down, not necessarily everything it \
                 did.\n",
            );
        }

        out
    }
}

/// Decides whether interrupted work may be restarted.
///
/// # Errors
///
/// Returns [`RecoveryDenied`] when the policy forbids it, the interruption is not retryable, or
/// the attempt limit is reached.
pub fn authorize_recovery(
    policy: RecoveryPolicy,
    interruption: &Interruption,
    attempts_so_far: u32,
    max_attempts: u32,
) -> Result<(), RecoveryDenied> {
    if !interruption.is_retryable() {
        return Err(RecoveryDenied::NotRetryable);
    }

    match policy {
        RecoveryPolicy::Never => return Err(RecoveryDenied::PolicyForbids),
        RecoveryPolicy::Manual => return Err(RecoveryDenied::NeedsAHuman),
        RecoveryPolicy::Automatic => {}
    }

    if attempts_so_far >= max_attempts {
        return Err(RecoveryDenied::OutOfAttempts { max_attempts });
    }

    Ok(())
}

/// Why interrupted work was not restarted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryDenied {
    /// The interruption was not the kind you restart through.
    #[error("the interruption was not retryable")]
    NotRetryable,
    /// The agent is configured never to restart.
    #[error("this agent's `recovery` policy is `never`")]
    PolicyForbids,
    /// The agent is configured to wait for a person.
    #[error("this agent's `recovery` policy is `manual`; a human decides")]
    NeedsAHuman,
    /// The work has already been restarted as often as it is allowed.
    #[error("already restarted {max_attempts} time(s); a crash loop is not resilience")]
    OutOfAttempts {
        /// The limit that was reached.
        max_attempts: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flight::{ItineraryId, Origin};

    fn flight() -> Flight {
        Flight::new(
            ItineraryId::generate(),
            Origin::Agent("analyst".into()),
            "developer".into(),
            "implement the retry policy",
            9,
        )
    }

    fn recovery() -> Recovery {
        Recovery {
            previous: RunId::from("run_01ABC"),
            interruption: Interruption::TowerRestart,
            attempt: 2,
        }
    }

    #[test]
    fn an_ordinary_dispatch_carries_no_briefing() {
        // A paragraph explaining that nothing happened would be noise in every prompt.
        let handover = Handover::dispatch(vec![flight()]);

        assert!(handover.brief().is_empty());
        assert!(!handover.cause.repeats_earlier_work());
    }

    #[test]
    fn a_recovered_run_is_told_what_happened_and_warned_about_side_effects() {
        let brief = Handover::recovered(recovery(), vec![flight()]).brief();

        assert!(brief.contains("continuing interrupted work"));
        assert!(brief.contains("run_01ABC"));
        assert!(brief.contains("the Tower restarted"));
        assert!(brief.contains("attempt 2"));
        assert!(
            brief.contains("Doing it twice is worse than doing it late"),
            "a restarted run must be warned before repeating a side effect"
        );
    }

    #[test]
    fn a_recovered_run_is_told_the_record_may_be_incomplete() {
        let brief = Handover::recovered(recovery(), vec![flight()]).with_progress(vec![
            "Read the work item".into(),
            "Wrote the failing test".into(),
        ]);

        let brief = brief.brief();
        assert!(brief.contains("- Read the work item"));
        assert!(brief.contains("- Wrote the failing test"));
        assert!(
            brief.contains("not necessarily everything it did"),
            "an interrupted run stops mid-sentence, and the brief must say so"
        );
    }

    #[test]
    fn a_recovered_run_with_no_record_is_told_that_too() {
        let brief = Handover::recovered(recovery(), vec![flight()]).brief();

        assert!(brief.contains("anywhere from nowhere to almost finished"));
    }

    #[test]
    fn a_steered_run_carries_the_instruction_and_its_precedence() {
        let brief = Handover::steered(
            Steer {
                previous: RunId::from("run_01XYZ"),
                note: "  Use the existing retry helper, do not write a new one.  ".to_owned(),
                at: SystemTime::now(),
            },
            vec![flight()],
        )
        .brief();

        assert!(brief.contains("A human has redirected"));
        assert!(brief.contains("> Use the existing retry helper"));
        assert!(
            brief.contains("takes precedence"),
            "steering that does not override the original request is just a suggestion"
        );
    }

    #[test]
    fn steering_and_recovery_both_repeat_earlier_work() {
        assert!(Cause::Recovered(recovery()).repeats_earlier_work());
        assert!(
            Cause::Steered(Steer {
                previous: RunId::from("run_1"),
                note: "stop".into(),
                at: SystemTime::now(),
            })
            .repeats_earlier_work()
        );
    }

    #[test]
    fn a_ground_stop_is_never_restarted_through() {
        // A factory that restarts through its own kill switch is not one anybody can stop.
        assert!(!Interruption::GroundStop.is_retryable());
        assert_eq!(
            authorize_recovery(RecoveryPolicy::Automatic, &Interruption::GroundStop, 0, 3),
            Err(RecoveryDenied::NotRetryable)
        );
    }

    #[test]
    fn ordinary_interruptions_are_retryable() {
        for interruption in [
            Interruption::TowerRestart,
            Interruption::Timeout,
            Interruption::Crashed { exit_code: Some(1) },
            Interruption::Crashed { exit_code: None },
        ] {
            assert!(interruption.is_retryable(), "{interruption}");
            assert_eq!(
                authorize_recovery(RecoveryPolicy::Automatic, &interruption, 0, 3),
                Ok(())
            );
        }
    }

    #[test]
    fn a_crash_loop_is_bounded() {
        assert_eq!(
            authorize_recovery(RecoveryPolicy::Automatic, &Interruption::Timeout, 3, 3),
            Err(RecoveryDenied::OutOfAttempts { max_attempts: 3 })
        );
        assert_eq!(
            authorize_recovery(RecoveryPolicy::Automatic, &Interruption::Timeout, 2, 3),
            Ok(())
        );
    }

    #[test]
    fn a_policy_of_never_or_manual_stops_automatic_restarts() {
        assert_eq!(
            authorize_recovery(RecoveryPolicy::Never, &Interruption::Timeout, 0, 3),
            Err(RecoveryDenied::PolicyForbids)
        );
        assert_eq!(
            authorize_recovery(RecoveryPolicy::Manual, &Interruption::Timeout, 0, 3),
            Err(RecoveryDenied::NeedsAHuman)
        );
        assert!(RecoveryPolicy::Automatic.is_automatic());
        assert!(!RecoveryPolicy::Manual.is_automatic());
    }

    #[test]
    fn a_zero_attempt_limit_disables_automatic_restarts_entirely() {
        assert_eq!(
            authorize_recovery(RecoveryPolicy::Automatic, &Interruption::Timeout, 0, 0),
            Err(RecoveryDenied::OutOfAttempts { max_attempts: 0 })
        );
    }

    #[test]
    fn the_flights_the_earlier_run_received_come_with_it() {
        // The new process remembers nothing, so it needs the work item again, not just a note
        // that one existed.
        let handover = Handover::recovered(recovery(), vec![flight()]);

        assert_eq!(handover.flights.len(), 1);
        assert_eq!(handover.flights[0].body, "implement the retry policy");
    }
}
