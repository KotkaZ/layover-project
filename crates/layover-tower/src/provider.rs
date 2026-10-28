//! Settling a run the model service ended: saying whose failure it was, and starting the work again
//! where the agent's `recovery` policy allows.
//!
//! A run that exits non-zero is the run's own failure — a test that failed, a command refused —
//! unless its CLI's events say the model service rejected or dropped its last call and nothing
//! after that shows the run carried on. See [`layover_core::provider`]. That run did not fail its
//! work; its work was interrupted. So it is treated exactly as a run interrupted by the Tower
//! going away: the agent's `recovery` policy and `max_recovery_attempts` decide, a Ground Stop
//! forbids it, and the restart is an ordinary run on the same flight that every rail applies to.
//!
//! Two things differ. The restart waits — a minute, then longer — because a provider fault often
//! comes in a cluster, and a restart into it spends an attempt on nothing. And it is told the model
//! service failed, so it knows the tools the earlier run called may well have acted.
//!
//! # A `manual` agent's run that did nothing
//!
//! `manual` says an agent's work must not happen twice. A run the service ended before it started
//! any tool did none of it — every side effect is a tool — so it is restarted all the same, on the
//! same attempts, back-off and rails. One that started a tool is left, as `manual` says. Left, it is
//! also put in front of a person as a help request: a failed run in history is not where anybody
//! looks, and on 28 October one such run took the only record of a pull request's follow-up with it.

use std::fmt::Write as _;

use jiff::{SignedDuration, Timestamp};
use layover_core::flight::{Flight, Origin};
use layover_core::handover::{
    Acted, ChildState, Interruption, Recovery, RecoveryPolicy, authorize_recovery, provider_backoff,
};
use layover_core::help::{Blocker, HelpRequest};
use layover_core::provider::ProviderFailure;
use layover_core::queue::Queued;

use crate::dispatcher::Ticket;
use crate::factory::Factory;

/// Work the model service interrupted, to be started again.
pub(crate) struct Restart {
    queued: Queued,
    /// Its agent's `recovery` is `manual`, and it is restarted only because the run it replaces
    /// started no tool.
    untouched: bool,
}

impl Factory {
    /// The model-service failure that ended a failed run, if its runner's output shows one.
    pub(crate) fn provider_failure(
        &self,
        runner: &str,
        transcript: &str,
    ) -> Option<ProviderFailure> {
        self.config
            .runners
            .get(runner)?
            .provider_reading()
            .classify(transcript)
    }

    /// How many tools a run started, when its runner's events can say.
    pub(crate) fn tools_started(&self, runner: &str, transcript: &str) -> Option<usize> {
        self.config
            .runners
            .get(runner)?
            .provider_reading()
            .tools_started(transcript)
    }

    /// The work to start again after the model service ended `ticket`'s run, or why it may not be.
    ///
    /// `tools` is how many tools the run started, where its events say.
    pub(crate) fn restart_after(
        &self,
        ticket: &Ticket<'_>,
        failure: &ProviderFailure,
        tools: Option<usize>,
    ) -> Result<Restart, String> {
        let queued = ticket
            .queued
            .as_ref()
            .ok_or_else(|| "the work it was given was not kept".to_owned())?;

        // Said as itself: a factory that restarts through its own kill switch is not one anybody
        // can stop, and whoever reads this should know that is why.
        if self.ground_stop_engaged() {
            return Err("a Ground Stop is engaged".to_owned());
        }

        let interruption = Interruption::ProviderError {
            request_id: failure.request_id.clone(),
        };
        let policy = ticket.authorised.agent.recovery;
        // Counted per flight, across every restart of it for whatever reason, so a provider
        // outage meets the same limit as a Tower that keeps going away.
        let attempt = queued.attempt();
        authorize_recovery(
            policy,
            &interruption,
            // The Tower watched the process exit.
            ChildState::Gone,
            if tools == Some(0) {
                Acted::Nothing
            } else {
                Acted::Maybe
            },
            attempt.saturating_sub(1),
            self.config.defaults.max_recovery_attempts,
        )
        .map_err(|denied| denied.to_string())?;

        // A new flight for the same work, in the same chain, from the same sender, so the route
        // check and the payload say what they said the first time.
        let original = &queued.flight;
        let flight = Flight::new(
            original.itinerary.clone(),
            original.from.clone(),
            original.to.clone(),
            original.body.clone(),
            original.hops_remaining,
        );
        let next = attempt.saturating_add(1);
        let wait = SignedDuration::try_from(provider_backoff(next)).unwrap_or(SignedDuration::ZERO);

        Ok(Restart {
            queued: Queued {
                flight,
                ..queued.clone()
            }
            .restarting(Recovery {
                previous: ticket.run.clone(),
                interruption,
                attempt: next,
            })
            .waiting_until(Timestamp::now() + wait),
            untouched: policy == RecoveryPolicy::Manual,
        })
    }

    /// Queues a restart the run's record has already promised, saying what became of it.
    ///
    /// After the record, and after the live mark is gone: a crash before this loses the restart,
    /// leaving a failed run somebody can see, where the other order could start the work twice —
    /// once from the queue and once from the live mark. But before the run stops counting as alive,
    /// so a schedule never sees its workflow idle between the two.
    pub(crate) fn queue_restart(&self, restart: Result<Restart, String>) -> String {
        match restart {
            Ok(restart) => {
                let said = restarting(&restart);
                match self.journal.queue(restart.queued) {
                    Ok(()) => format!("the model service failed; {said}"),
                    Err(error) => format!(
                        "the model service failed; the restart could not be queued: {error}"
                    ),
                }
            }
            Err(why) => format!("the model service failed; not restarted: {why}"),
        }
    }

    /// Puts a `manual` agent's run that the model service ended, and that was not restarted, in
    /// front of a person: a help request, not fatal, naming the run, what the service said, what
    /// the run had done first and any layover it had collected.
    ///
    /// Best-effort: the run's record already says it was not restarted, and a journal that cannot
    /// be written to is no reason to lose that.
    pub(crate) fn ask_about(
        &self,
        ticket: &Ticket<'_>,
        failure: &ProviderFailure,
        why: &str,
        tools: Option<usize>,
    ) {
        if ticket.authorised.agent.recovery != RecoveryPolicy::Manual {
            return;
        }

        let agent = &ticket.authorised.name;
        let failed = failure.to_string();
        let said = layover_core::help::redact::secrets(&failed);
        let mut detail = format!(
            "The model service ended run {} of `{agent}`, in chain {}, and it was not restarted: \
             {why}.\n\nWhat the service said: {said}.",
            ticket.run,
            ticket.chain.as_str(),
        );
        if let Some(id) = &failure.request_id {
            let _ = write!(detail, "\nThe provider's request id: {id}.");
        }
        let _ = match tools {
            Some(0) => write!(
                detail,
                "\n\nBefore that it had started no tool, so nothing it did needs checking."
            ),
            Some(count) => write!(
                detail,
                "\n\nBefore that it had started {count} tool{}, so some of its work may have \
                 taken effect: check before repeating any of it.",
                if count == 1 { "" } else { "s" }
            ),
            None => write!(
                detail,
                "\n\nIts runner's events do not say what tools it started, so some of its work may \
                 have taken effect: check before repeating any of it."
            ),
        };
        if let Some(waiting_for) = self.collected_layover(ticket) {
            let _ = write!(
                detail,
                "\n\nIt had collected the layover booked for \"{waiting_for}\", and nothing has \
                 booked it again: that is no longer being followed."
            );
        }
        let _ = write!(
            detail,
            "\n\nReply to start `{agent}` again, in a new chain with this one's workflow and flags, \
             with what it should do; or resolve this once the work is dealt with another way."
        );

        let request = HelpRequest::new(
            agent.clone(),
            ticket.run.clone(),
            ticket.chain.clone(),
            Blocker::Environment,
            format!("not restarted after the model service failed: {why}"),
            detail,
            Timestamp::now(),
        )
        .raised_in(
            self.chains.scope_of(&ticket.chain),
            self.recorded_flags(&ticket.chain),
        )
        .chosen_as(self.chains.chosen_of(&ticket.chain));
        let _ = self.journal.ask(&request);
    }

    /// What the layover a run was started to collect was waiting for, when it was started by one.
    fn collected_layover(&self, ticket: &Ticket<'_>) -> Option<String> {
        let Origin::Resumed(id) = &ticket.queued.as_ref()?.flight.from else {
            return None;
        };
        self.journal
            .layovers()
            .ok()?
            .into_iter()
            .find(|layover| layover.id == *id)
            .map(|layover| layover.waiting_for)
    }
}

/// The line a provider-ended run's record carries: how it exited, what the service said, and
/// what is being done about it.
pub(crate) fn detail(
    exit_code: Option<i32>,
    failure: &ProviderFailure,
    restart: &Result<Restart, String>,
) -> String {
    let how = exit_code.map_or_else(
        || "exited abnormally".to_owned(),
        |code| format!("exited with code {code}"),
    );
    let said = failure.to_string();
    let failure = layover_core::help::redact::secrets(&said);
    let then = match restart {
        Ok(restart) => restarting(restart),
        Err(why) => format!("not restarted: {why}"),
    };
    format!("{how}: {failure}; {then}")
}

/// "restarting as attempt 2 after a 1-minute wait", and why, when it is restarted although its
/// agent's `recovery` is `manual`.
fn restarting(restart: &Restart) -> String {
    let attempt = restart.queued.attempt();
    let minutes = provider_backoff(attempt).as_secs() / 60;
    let when = format!("as attempt {attempt} after a {minutes}-minute wait");
    if restart.untouched {
        format!(
            "it started no tool, so restarting although this agent's `recovery` policy is \
             `manual`, {when}"
        )
    } else {
        format!("restarting {when}")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use layover_core::config::Config;
    use layover_core::flight::{ItineraryId, Origin, RunId};
    use layover_core::itinerary::Itinerary;

    use super::*;

    const FACTORY: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "copilot"

[runners.copilot]
cli = "copilot"

[agents.wolf]
prompt = "verify"
entry = true

[agents.mailman]
prompt = "post"
entry = true
recovery = "manual"
"#;

    struct Temp(std::path::PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_ground_stop_or_work_that_was_not_kept_stops_the_restart() {
        // Also of a `manual` agent's run that did nothing: that it may be restarted is about the
        // agent's policy, never about the kill switch.
        for (agent, tools) in [("wolf", None), ("mailman", Some(0))] {
            let temp = Temp(std::env::temp_dir().join(format!(
                "layover-provider-unit-{agent}-{}",
                std::process::id()
            )));
            let config: Config = toml::from_str(FACTORY).expect("parses");
            let factory = Factory::new(config, &temp.0).expect("opens");
            let mut ticket = ticket(&factory, agent);
            let failure = ProviderFailure::default();
            assert!(
                factory.restart_after(&ticket, &failure, tools).is_ok(),
                "{agent}"
            );

            std::fs::write(temp.0.join(".layover").join("ground-stop"), "").expect("engages");
            assert_eq!(
                factory
                    .restart_after(&ticket, &failure, tools)
                    .err()
                    .as_deref(),
                Some("a Ground Stop is engaged"),
                "{agent}"
            );

            std::fs::remove_file(temp.0.join(".layover").join("ground-stop")).expect("releases");
            ticket.queued = None;
            assert_eq!(
                factory
                    .restart_after(&ticket, &failure, tools)
                    .err()
                    .as_deref(),
                Some("the work it was given was not kept"),
                "{agent}"
            );
        }
    }

    #[test]
    fn a_manual_agent_s_run_that_started_a_tool_or_cannot_say_is_left() {
        let temp = Temp(std::env::temp_dir().join(format!(
            "layover-provider-unit-acted-{}",
            std::process::id()
        )));
        let config: Config = toml::from_str(FACTORY).expect("parses");
        let factory = Factory::new(config, &temp.0).expect("opens");
        let ticket = ticket(&factory, "mailman");

        for tools in [Some(1), None] {
            assert_eq!(
                factory
                    .restart_after(&ticket, &ProviderFailure::default(), tools)
                    .err()
                    .as_deref(),
                Some("this agent's `recovery` policy is `manual`; a human decides"),
                "{tools:?}"
            );
        }
    }

    /// The ticket of a run of `agent`, started by a person.
    fn ticket<'a>(factory: &'a Factory, agent: &str) -> Ticket<'a> {
        let chain = ItineraryId::generate();
        let queued = Queued::new(
            Flight::new(chain.clone(), Origin::Human, agent.into(), "verify", 6),
            None,
            BTreeMap::new(),
        );
        let authorised = crate::dispatch::authorise(
            factory.config(),
            factory.graph(),
            &Itinerary::new(chain.clone(), 6, 5.0, 10),
            None,
            &queued.flight,
            false,
        )
        .expect("authorised");
        Ticket {
            run: RunId::generate(),
            chain,
            authorised,
            started_at: Timestamp::now(),
            queued_at: None,
            sent_by: None,
            selection: layover_core::config::Selection::default(),
            queued: Some(queued),
        }
    }
}
