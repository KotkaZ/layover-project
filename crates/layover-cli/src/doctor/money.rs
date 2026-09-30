//! What `layover doctor` says about the factory's money: whether the Reserve is refusing work.
//!
//! A Reserve refusal is the rail doing its job, so it is a warning rather than a fault — but it is
//! work somebody scheduled that did not happen, and a factory that stopped for that reason must not
//! look the same as one that stopped for none.

use std::path::Path;

use jiff::Timestamp;
use layover_core::config::Config;
use layover_core::run::{Outcome, RunRecord};
use layover_tower::reserve;

use super::{Finding, Report, Severity};

/// Whether the Reserve refused work in the window, and whether it would refuse a run due now.
///
/// One finding rather than two: an operator who has just been told the Reserve refused six runs
/// needs to know next whether it still would, not to read the same reason twice.
pub(super) fn reserve(config: &Config, root: &Path, runs: &[RunRecord], report: &mut Report) {
    let refused: Vec<&RunRecord> = runs
        .iter()
        .filter(|record| {
            record.outcome == Outcome::Halted
                && record
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.starts_with(reserve::REFUSED))
        })
        .collect();
    let latest = refused.iter().max_by_key(|record| record.started_at);
    let history = root.join(".layover").join("history");
    let now = reserve::refusal(config, &history, Timestamp::now());

    let (summary, advice) = match (latest, now) {
        (None, None) => return,
        (None, Some(why)) => (
            "the Reserve is exhausted: nothing new will start".to_owned(),
            why,
        ),
        (Some(_), Some(why)) => (
            format!(
                "the Reserve refused {} run(s), and is still exhausted",
                refused.len()
            ),
            why,
        ),
        (Some(latest), None) => (
            format!("the Reserve refused {} run(s)", refused.len()),
            format!(
                "It has room again now. Most recently: {}",
                latest.detail.as_deref().unwrap_or_default()
            ),
        ),
    };

    report.findings.push(Finding {
        severity: Severity::Warning,
        summary,
        advice: format!(
            "{advice} Only measured spend counts against it — dollars a runner printed, and \
             Copilot credits."
        ),
    });
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;
    use layover_core::agent::AgentName;
    use layover_core::cost::{CostSource, TokenUsage};
    use layover_core::flight::{ItineraryId, RunId};
    use layover_store::History;

    use super::*;

    fn run(source: CostSource, usd: f64, outcome: Outcome, detail: Option<&str>) -> RunRecord {
        let at = Timestamp::now()
            .checked_sub(SignedDuration::from_hours(1))
            .expect("in range");
        let mut record = RunRecord::started(
            RunId::generate(),
            ItineraryId::generate(),
            AgentName::new("reviewer"),
            at,
        )
        .finished(outcome, at)
        .costing(usd, source, TokenUsage::default());
        record.detail = detail.map(ToOwned::to_owned);
        record
    }

    #[test]
    fn copilot_runs_priced_from_their_credits_are_not_counted_as_silent() {
        let runs: Vec<_> = (0..4)
            .map(|_| run(CostSource::CopilotCredits, 15.11, Outcome::Succeeded, None))
            .collect();
        let mut report = Report::default();
        super::super::unreported_costs(&runs, &mut report);

        assert!(report.findings.is_empty(), "{:?}", report.findings);
    }

    #[test]
    fn a_copilot_run_killed_before_it_reported_is_still_silent() {
        let runs = vec![
            run(CostSource::CopilotCredits, 15.11, Outcome::Succeeded, None),
            run(CostSource::Unreported, 0.0, Outcome::TimedOut, None),
        ];
        let mut report = Report::default();
        super::super::unreported_costs(&runs, &mut report);

        assert_eq!(report.findings.len(), 1);
        assert!(report.findings[0].summary.starts_with("1 of 2"));
    }

    /// A factory root whose history holds `runs`, cleaned up on drop.
    struct Root(std::path::PathBuf);

    impl Root {
        fn with(tag: &str, runs: &[RunRecord]) -> Self {
            let root = std::env::temp_dir().join(format!(
                "layover-doctor-reserve-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let history = History::open(root.join(".layover").join("history")).expect("opens");
            for record in runs {
                history.append(record).expect("appends");
            }
            Self(root)
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn capped(cap: f64) -> Config {
        toml::from_str(&format!(
            "[layover]\nwork_dir = \"work\"\n\n[reserve]\nfuel_usd = {cap}\nwindow_hours = 24\n"
        ))
        .expect("parses")
    }

    fn refusal() -> RunRecord {
        let why = format!(
            "{} — $30.21 of $20.00 spent in the last 24h.",
            reserve::REFUSED
        );
        run(CostSource::Reported, 0.0, Outcome::Halted, Some(&why))
    }

    fn check(config: &Config, runs: &[RunRecord], tag: &str) -> Report {
        let root = Root::with(tag, runs);
        let mut report = Report::default();
        reserve(config, &root.0, runs, &mut report);
        report
    }

    #[test]
    fn a_refusal_while_the_reserve_is_still_full_is_one_warning_with_the_current_reason() {
        let runs = vec![
            run(CostSource::CopilotCredits, 30.21, Outcome::Succeeded, None),
            refusal(),
            run(
                CostSource::Reported,
                0.0,
                Outcome::Halted,
                Some("halted by a Ground Stop"),
            ),
        ];
        let report = check(&capped(20.0), &runs, "full");

        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        let finding = &report.findings[0];
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(
            finding.summary,
            "the Reserve refused 1 run(s), and is still exhausted"
        );
        assert!(
            finding.advice.contains("$30.21 of $20.00"),
            "{}",
            finding.advice
        );
    }

    #[test]
    fn a_refusal_after_which_the_reserve_has_room_says_so() {
        let report = check(&capped(1_000.0), &[refusal()], "room");

        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].summary, "the Reserve refused 1 run(s)");
        assert!(
            report.findings[0]
                .advice
                .starts_with("It has room again now.")
        );
    }

    #[test]
    fn an_exhausted_reserve_is_reported_before_anything_is_refused() {
        let runs = [run(
            CostSource::CopilotCredits,
            25.0,
            Outcome::Succeeded,
            None,
        )];

        let report = check(&capped(20.0), &runs, "before");
        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        assert_eq!(
            report.findings[0].summary,
            "the Reserve is exhausted: nothing new will start"
        );
        assert!(report.findings[0].advice.contains("$25.00 of $20.00"));

        let unlimited: Config =
            toml::from_str("[layover]\nwork_dir = \"work\"\n\n[reserve]\nfuel_usd = 0\n")
                .expect("parses");
        assert!(check(&unlimited, &runs, "unlimited").findings.is_empty());
    }
}
