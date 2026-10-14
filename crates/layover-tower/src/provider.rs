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

use jiff::{SignedDuration, Timestamp};
use layover_core::flight::Flight;
use layover_core::handover::{
    ChildState, Interruption, Recovery, authorize_recovery, provider_backoff,
};
use layover_core::provider::ProviderFailure;
use layover_core::queue::Queued;

use crate::dispatcher::Ticket;
use crate::factory::Factory;

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

    /// The work to start again after the model service ended `ticket`'s run, or why it may not be.
    pub(crate) fn restart_after(
        &self,
        ticket: &Ticket<'_>,
        failure: &ProviderFailure,
    ) -> Result<Queued, String> {
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
        // Counted per flight, across every restart of it for whatever reason, so a provider
        // outage meets the same limit as a Tower that keeps going away.
        let attempt = queued.attempt();
        authorize_recovery(
            ticket.authorised.agent.recovery,
            &interruption,
            // The Tower watched the process exit.
            ChildState::Gone,
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

        Ok(Queued {
            flight,
            ..queued.clone()
        }
        .restarting(Recovery {
            previous: ticket.run.clone(),
            interruption,
            attempt: next,
        })
        .waiting_until(Timestamp::now() + wait))
    }

    /// Queues a restart the run's record has already promised, saying what became of it.
    ///
    /// After the record, and after the live mark is gone: a crash before this loses the restart,
    /// leaving a failed run somebody can see, where the other order could start the work twice —
    /// once from the queue and once from the live mark. But before the run stops counting as alive,
    /// so a schedule never sees its workflow idle between the two.
    pub(crate) fn queue_restart(&self, restart: Result<Queued, String>) -> String {
        match restart {
            Ok(queued) => {
                let said = restarting(&queued);
                match self.journal.queue(queued) {
                    Ok(()) => format!("the model service failed; {said}"),
                    Err(error) => format!(
                        "the model service failed; the restart could not be queued: {error}"
                    ),
                }
            }
            Err(why) => format!("the model service failed; not restarted: {why}"),
        }
    }
}

/// The line a provider-ended run's record carries: how it exited, what the service said, and
/// what is being done about it.
pub(crate) fn detail(
    exit_code: Option<i32>,
    failure: &ProviderFailure,
    restart: &Result<Queued, String>,
) -> String {
    let how = exit_code.map_or_else(
        || "exited abnormally".to_owned(),
        |code| format!("exited with code {code}"),
    );
    let said = failure.to_string();
    let failure = layover_core::help::redact::secrets(&said);
    let then = match restart {
        Ok(queued) => restarting(queued),
        Err(why) => format!("not restarted: {why}"),
    };
    format!("{how}: {failure}; {then}")
}

/// "restarting as attempt 2 after a 1-minute wait".
fn restarting(queued: &Queued) -> String {
    let minutes = provider_backoff(queued.attempt()).as_secs() / 60;
    format!(
        "restarting as attempt {} after a {minutes}-minute wait",
        queued.attempt()
    )
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
"#;

    struct Temp(std::path::PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_ground_stop_or_work_that_was_not_kept_stops_the_restart() {
        let temp = Temp(
            std::env::temp_dir().join(format!("layover-provider-unit-{}", std::process::id())),
        );
        let config: Config = toml::from_str(FACTORY).expect("parses");
        let factory = Factory::new(config, &temp.0).expect("opens");
        let chain = ItineraryId::generate();
        let queued = Queued::new(
            Flight::new(chain.clone(), Origin::Human, "wolf".into(), "verify", 6),
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
        let mut ticket = Ticket {
            run: RunId::generate(),
            chain,
            authorised,
            started_at: Timestamp::now(),
            queued_at: None,
            sent_by: None,
            selection: layover_core::config::Selection::default(),
            queued: Some(queued),
        };
        let failure = ProviderFailure::default();
        assert!(factory.restart_after(&ticket, &failure).is_ok());

        std::fs::write(temp.0.join(".layover").join("ground-stop"), "").expect("engages");
        assert_eq!(
            factory.restart_after(&ticket, &failure).err().as_deref(),
            Some("a Ground Stop is engaged")
        );

        std::fs::remove_file(temp.0.join(".layover").join("ground-stop")).expect("releases");
        ticket.queued = None;
        assert_eq!(
            factory.restart_after(&ticket, &failure).err().as_deref(),
            Some("the work it was given was not kept")
        );
    }
}
