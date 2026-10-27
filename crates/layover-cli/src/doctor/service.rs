//! What `layover doctor` says about agents the model service keeps ending.
//!
//! A run the model service ended is restarted, a few times, with waits between. That rides out an
//! outage. It cannot fix a model the CLI does not have at all — a name mistyped, a model not in the
//! plan — which fails the same way on every attempt, looks to the Tower exactly like an outage, and
//! once the attempts run out leaves the work waiting for a person who has not been told. So an
//! agent whose last few runs were *all* ended by the model service is named, with what the CLI said
//! the last time, which is what tells a person which of the two it was.

use std::collections::BTreeMap;

use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::run::{FailureCause, RunRecord};

use super::{Finding, Report, Severity};

/// Names every agent whose most recent runs were all ended by the model service.
///
/// As many as one piece of work may take — the run and every restart `max_recovery_attempts`
/// allows — and never fewer than two: by then at least one piece of work has run out of attempts,
/// or the agent is not restarted at all and the work is already waiting.
pub(super) fn ended_by_service(config: &Config, runs: &[RunRecord], report: &mut Report) {
    let enough = usize::try_from(config.defaults.max_recovery_attempts)
        .unwrap_or(usize::MAX)
        .saturating_add(1)
        .max(2);

    let mut by_agent: BTreeMap<&AgentName, Vec<&RunRecord>> = BTreeMap::new();
    for run in runs {
        by_agent.entry(&run.agent).or_default().push(run);
    }

    for (agent, mut theirs) in by_agent {
        theirs.sort_by_key(|run| std::cmp::Reverse(run.finished_at.unwrap_or(run.started_at)));
        let ended: Vec<&&RunRecord> = theirs
            .iter()
            .take_while(|run| run.cause == Some(FailureCause::ProviderError))
            .collect();
        if ended.len() < enough {
            continue;
        }

        let latest = ended[0];
        let said = latest.detail.as_deref().unwrap_or("nothing");
        let on = latest
            .model
            .as_deref()
            .map_or_else(String::new, |model| format!(" on `{model}`"));
        report.findings.push(Finding {
            severity: Severity::Warning,
            summary: format!(
                "`{agent}`'s last {} runs{on} were all ended by the model service",
                ended.len()
            ),
            advice: format!(
                "The latest record says: {said}. An outage passes, and the restarts ride it out; a \
                 model the CLI does not have fails like this every time. If it says the model is \
                 not available and other runs on it went through meanwhile, check the agent's \
                 `model`. Either way, work these runs carried may be waiting: continue its chain \
                 from the dashboard."
            ),
        });
    }
}

#[cfg(test)]
mod tests {
    use jiff::{SignedDuration, Timestamp};
    use layover_core::run::Outcome;

    use super::*;

    fn config(max_recovery_attempts: u32) -> Config {
        toml::from_str(&format!(
            "[defaults]\nmax_recovery_attempts = {max_recovery_attempts}\n\
             [agents.tester]\nprompt = \"test\"\nentry = true\n"
        ))
        .expect("parses")
    }

    fn run(minutes_ago: i64, by_service: bool) -> RunRecord {
        let at = Timestamp::now() - SignedDuration::from_mins(minutes_ago);
        let mut record = RunRecord::started(
            layover_core::RunId::generate(),
            layover_core::flight::ItineraryId::generate(),
            AgentName::new("tester"),
            at,
        )
        .finished(
            if by_service {
                Outcome::Failed
            } else {
                Outcome::Succeeded
            },
            at,
        );
        record.model = Some("gpt-6.1-sol".to_owned());
        if by_service {
            record.cause = Some(FailureCause::ProviderError);
            record.detail = Some(
                "exited with code 1: the model service failed: Model \"gpt-6.1-sol\" from \
                 --model flag is not available.; not restarted: this agent has already \
                 restarted 2 time(s)"
                    .to_owned(),
            );
        }
        record
    }

    fn findings(config: &Config, runs: &[RunRecord]) -> Vec<Finding> {
        let mut report = Report::default();
        ended_by_service(config, runs, &mut report);
        report.findings
    }

    #[test]
    fn an_agent_whose_every_attempt_the_service_ended_is_named_with_what_the_cli_said() {
        let runs = [run(60, false), run(20, true), run(15, true), run(1, true)];
        let found = findings(&config(2), &runs);

        let [finding] = &found[..] else {
            panic!("one finding: {found:?}");
        };
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(
            finding.summary,
            "`tester`'s last 3 runs on `gpt-6.1-sol` were all ended by the model service"
        );
        assert!(
            finding
                .advice
                .contains("from --model flag is not available"),
            "{}",
            finding.advice
        );
    }

    #[test]
    fn a_run_that_went_through_since_ends_the_streak() {
        // An outage the restarts rode out: the latest run went through, so nothing is waiting.
        let runs = [run(20, true), run(15, true), run(10, true), run(1, false)];

        assert!(findings(&config(2), &runs).is_empty());
        assert!(
            findings(&config(2), &runs[..2]).is_empty(),
            "two in a row is still within one piece of work's three attempts"
        );
        assert_eq!(
            findings(&config(0), &runs[..2]).len(),
            1,
            "with no restarts, two in a row is never ridden out"
        );
    }
}
