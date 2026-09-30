//! What `layover doctor` says about work that waited while a slot was free.
//!
//! The symptom a Tower that ran one agent at a time produced without a word: `max_concurrent_runs`
//! at four, ten reviews queued, and each starting only when the one before it ended — hours late,
//! with three slots idle the whole time. A busy factory makes work wait too, so waiting is not the
//! finding. Waiting *with a slot free* is: nothing was stopping the work but the thing meant to
//! start it.

use std::path::Path;

use jiff::{SignedDuration, Timestamp};
use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::queue::Queued;
use layover_core::run::RunRecord;

use super::{Finding, Report, Severity};

/// Work that waited longer than `timeout_sec` while a slot was free: in history, and in the queue
/// now.
pub(super) fn waits(
    config: &Config,
    root: &Path,
    runs: &[RunRecord],
    pending: &[Queued],
    report: &mut Report,
) {
    let limit =
        SignedDuration::from_secs(i64::try_from(config.defaults.timeout_sec).unwrap_or(i64::MAX));

    let waited: Vec<(&RunRecord, SignedDuration)> = runs
        .iter()
        .filter_map(|run| {
            let free = free_while_waiting(config, run, runs)?;
            (free > limit).then_some((run, free))
        })
        .collect();

    if let Some((worst, free)) = waited.iter().max_by_key(|(_, free)| *free) {
        report.findings.push(Finding {
            severity: Severity::Warning,
            summary: format!(
                "{} run(s) waited longer than `timeout_sec` while a slot was free",
                waited.len()
            ),
            advice: format!(
                "`{}` ({}) waited {} with fewer than `max_concurrent_runs` = {} runs alive. \
                 Nothing was stopping it but the thing meant to start it: the Tower was not \
                 running, a Ground Stop was engaged, or the factory was run by a release that \
                 started one agent at a time.",
                worst.agent,
                worst.run,
                hours(*free),
                config.defaults.max_concurrent_runs.max(1)
            ),
        });
    }

    queued_now(config, root, pending, limit, report);
}

/// How long `run` spent queued while a slot it could have used was free, or `None` when history
/// does not say when it was queued.
///
/// Free means fewer runs alive than the factory's limit and, when the agent has a cap of its own,
/// fewer of that agent's runs than its cap. Measured from the other runs history records, so a run
/// still alive now is not counted — which can only make this say less.
fn free_while_waiting(
    config: &Config,
    run: &RunRecord,
    runs: &[RunRecord],
) -> Option<SignedDuration> {
    let queued = run.queued_at?;
    let started = run.started_at;
    if started <= queued {
        return None;
    }

    let overlapping: Vec<&RunRecord> = runs
        .iter()
        .filter(|other| other.run != run.run)
        .filter(|other| {
            other.started_at < started && other.finished_at.is_none_or(|end| end > queued)
        })
        .collect();

    // Every moment in the wait at which the count of runs alive can change.
    let mut edges = vec![queued, started];
    for other in &overlapping {
        edges.push(other.started_at.max(queued));
        if let Some(end) = other.finished_at {
            edges.push(end.min(started));
        }
    }
    edges.sort();
    edges.dedup();

    let global = config.defaults.max_concurrent_runs.max(1);
    let own = config
        .agents
        .get(&run.agent)
        .and_then(|agent| agent.max_concurrent);

    let mut free = SignedDuration::ZERO;
    for pair in edges.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let alive: Vec<&&RunRecord> = overlapping
            .iter()
            .filter(|other| {
                other.started_at <= from && other.finished_at.is_none_or(|end| end > from)
            })
            .collect();
        let mine = alive
            .iter()
            .filter(|other| other.agent == run.agent)
            .count();
        if alive.len() < global && own.is_none_or(|cap| mine < cap) {
            free += to.duration_since(from);
        }
    }
    Some(free)
}

/// Flights queued longer than `timeout_sec` while fewer runs are alive than the limit.
///
/// Said separately from history, because this is the one a person can still do something about.
fn queued_now(
    config: &Config,
    root: &Path,
    pending: &[Queued],
    limit: SignedDuration,
    report: &mut Report,
) {
    // A Ground Stop holds everything on purpose, and has a finding of its own.
    if root.join(".layover").join("ground-stop").exists() {
        return;
    }

    let alive = alive_now(root);
    if alive.len() >= config.defaults.max_concurrent_runs.max(1) {
        return;
    }

    let now = Timestamp::now();
    let stuck: Vec<&Queued> = pending
        .iter()
        .filter(|queued| now.duration_since(queued.flight.sent_at) > limit)
        .filter(|queued| {
            // A flight for an agent at its own cap is waiting for that agent, as asked.
            let cap = config
                .agents
                .get(&queued.flight.to)
                .and_then(|agent| agent.max_concurrent);
            let running = alive
                .iter()
                .filter(|agent| **agent == queued.flight.to)
                .count();
            cap.is_none_or(|cap| running < cap)
        })
        .collect();

    let Some(oldest) = stuck.iter().min_by_key(|queued| queued.flight.sent_at) else {
        return;
    };
    report.findings.push(Finding {
        severity: Severity::Warning,
        summary: format!(
            "{} flight(s) have been queued longer than `timeout_sec` with only {} of {} runs alive",
            stuck.len(),
            alive.len(),
            config.defaults.max_concurrent_runs.max(1)
        ),
        advice: format!(
            "The oldest, for `{}`, was queued {} ago. A slot is free, so nothing should be holding \
             it: check that `layover serve` is running for this factory — the dashboard names the \
             Tower running the queue — or start it with `layover run`.",
            oldest.flight.to,
            hours(now.duration_since(oldest.flight.sent_at))
        ),
    });
}

/// The agents of every run recorded as alive now.
fn alive_now(root: &Path) -> Vec<AgentName> {
    let dir = root.join(".layover").join("state").join("runs");
    // Looked for before opening, so a doctor run against a factory that never ran creates nothing.
    if !dir.is_dir() {
        return Vec::new();
    }
    layover_tower::Ledger::open(dir)
        .and_then(|ledger| ledger.live())
        .map(|live| live.into_iter().map(|run| run.agent).collect())
        .unwrap_or_default()
}

fn hours(duration: SignedDuration) -> String {
    let minutes = duration.as_secs() / 60;
    if minutes < 60 {
        format!("{minutes} minute(s)")
    } else {
        format!("{:.1} hour(s)", duration.as_secs_f64() / 3_600.0)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use layover_core::flight::{Flight, ItineraryId, Origin, RunId};

    use super::*;

    fn config(extra: &str) -> Config {
        toml::from_str(&format!(
            r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"
timeout_sec = 3600
max_concurrent_runs = 4

[runners.shell]
command = ["echo"]

[agents.eagle]
prompt = "review"
entry = true

[agents.mailman]
prompt = "post"
entry = true
{extra}
"#
        ))
        .expect("parses")
    }

    fn ago(hours: f64) -> Timestamp {
        #[expect(clippy::cast_possible_truncation, reason = "whole seconds are plenty")]
        let seconds = (hours * 3_600.0) as i64;
        Timestamp::now() - SignedDuration::from_secs(seconds)
    }

    /// A run of `agent`, queued `queued` hours ago and alive from `from` to `to` hours ago.
    fn run(agent: &str, queued: Option<f64>, from: f64, to: f64) -> RunRecord {
        let mut record = RunRecord::started(
            RunId::generate(),
            ItineraryId::generate(),
            AgentName::new(agent),
            ago(from),
        );
        record.queued_at = queued.map(ago);
        record.finished_at = Some(ago(to));
        record.outcome = layover_core::run::Outcome::Succeeded;
        record
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("layover-waits-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn reviews_that_queued_behind_one_another_with_slots_free_are_a_warning() {
        // The 1.3.0 sweep: queued together, run one after another, three slots idle throughout.
        let runs = vec![
            run("eagle", Some(4.0), 4.0, 3.0),
            run("eagle", Some(4.0), 3.0, 2.0),
            run("eagle", Some(4.0), 2.0, 1.0),
        ];
        let mut report = Report::default();

        waits(&config(""), &temp("sweep"), &runs, &[], &mut report);

        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        assert_eq!(report.findings[0].severity, Severity::Warning);
        assert!(
            report.findings[0].summary.starts_with("1 run(s)"),
            "{:?}",
            report.findings
        );
        assert!(
            report.findings[0].advice.contains("2.0 hour(s)"),
            "{:?}",
            report.findings
        );
    }

    #[test]
    fn waiting_while_every_slot_was_busy_is_not_a_finding() {
        let mut runs: Vec<RunRecord> = (0..4).map(|_| run("eagle", Some(5.0), 5.0, 1.0)).collect();
        runs.push(run("eagle", Some(5.0), 1.0, 0.5));
        let mut report = Report::default();

        waits(&config(""), &temp("busy"), &runs, &[], &mut report);

        assert!(report.findings.is_empty(), "{:?}", report.findings);
    }

    #[test]
    fn waiting_for_an_agent_at_its_own_cap_is_not_a_finding() {
        let runs = vec![
            run("mailman", Some(4.0), 4.0, 2.0),
            run("mailman", Some(4.0), 2.0, 1.0),
        ];
        let mut report = Report::default();

        waits(
            &config("max_concurrent = 1"),
            &temp("capped"),
            &runs,
            &[],
            &mut report,
        );

        assert!(report.findings.is_empty(), "{:?}", report.findings);
    }

    #[test]
    fn a_short_wait_and_a_run_that_says_nothing_about_its_queueing_are_not_findings() {
        let runs = vec![
            run("eagle", Some(1.5), 1.0, 0.5),
            run("eagle", None, 3.0, 0.1),
        ];
        let mut report = Report::default();

        waits(&config(""), &temp("short"), &runs, &[], &mut report);

        assert!(report.findings.is_empty(), "{:?}", report.findings);
    }

    fn queued(agent: &str, hours: f64) -> Queued {
        let mut flight = Flight::new(
            ItineraryId::generate(),
            Origin::Human,
            AgentName::new(agent),
            "go",
            4,
        );
        flight.sent_at = ago(hours);
        Queued::new(flight, None, BTreeMap::new())
    }

    #[test]
    fn work_queued_for_hours_with_nothing_running_is_a_warning_unless_grounded() {
        let root = temp("pending");
        let pending = vec![queued("eagle", 2.0), queued("eagle", 0.1)];

        let mut report = Report::default();
        waits(&config(""), &root, &[], &pending, &mut report);
        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
        assert!(
            report.findings[0].summary.contains("0 of 4"),
            "{:?}",
            report.findings
        );

        std::fs::create_dir_all(root.join(".layover")).expect("dirs");
        std::fs::write(root.join(".layover").join("ground-stop"), "").expect("engages");
        let mut grounded = Report::default();
        waits(&config(""), &root, &[], &pending, &mut grounded);
        assert!(
            grounded.findings.is_empty(),
            "a Ground Stop holds work on purpose"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
