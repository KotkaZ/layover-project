//! Checking whether a factory is actually healthy, rather than merely running.
//!
//! # Why this exists
//!
//! The claim this project makes is *lights-out*: it runs for days with nobody watching. The
//! failures that matter are therefore the ones nobody is there to see, and almost all of them look
//! like nothing from the outside — a chain that stalled with every run reporting success, a
//! schedule skipping every tick because its work overruns, a cost total built from silence, a
//! learning nobody will ever confirm.
//!
//! "Did the soak pass?" should not be a judgement call made by squinting at a dashboard at the end
//! of two days. This turns it into a command with a verdict.
//!
//! # What it deliberately does not do
//!
//! It does not look at whether agents did *good work*. That is unknowable from here and is the
//! operator's job. Everything checked is a property of the supervisor: work that went missing,
//! money that cannot be accounted for, rails that did not hold.

use std::fmt::Write as _;
use std::path::Path;

use jiff::Timestamp;
use layover_core::config::Config;
use layover_core::cost::{CostSource, Span, Window};
use layover_core::run::Outcome;
use layover_store::{HelpFilter, History, Journal, RunFilter};

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Worth knowing, not worth stopping for.
    Note,
    /// Something is wrong and a person should look.
    Warning,
    /// Work was lost, or money cannot be accounted for.
    Fault,
}

impl Severity {
    const fn label(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Warning => "warning",
            Self::Fault => "fault",
        }
    }
}

/// One thing the check found.
#[derive(Debug, Clone)]
pub struct Finding {
    /// How serious.
    pub severity: Severity,
    /// What was found, in one line.
    pub summary: String,
    /// What to do about it.
    pub advice: String,
}

/// Everything a check turned up.
#[derive(Debug, Default)]
pub struct Report {
    /// What was found, worst first.
    pub findings: Vec<Finding>,
    /// How many runs were examined.
    pub runs: usize,
    /// How long the factory has been accumulating history, in hours.
    pub hours: i64,
}

impl Report {
    /// Whether anything found would fail an unattended soak.
    #[must_use]
    pub fn healthy(&self) -> bool {
        !self
            .findings
            .iter()
            .any(|finding| finding.severity >= Severity::Warning)
    }

    /// The report as a person would read it.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();

        let _ = writeln!(
            out,
            "Examined {} run(s) over {} hour(s).\n",
            self.runs, self.hours
        );

        if self.findings.is_empty() {
            out.push_str(
                "Nothing to report. Every run is accounted for, every chain reached an \
                          end, and every figure is measured.\n",
            );
            return out;
        }

        for finding in &self.findings {
            let _ = writeln!(out, "{}: {}", finding.severity.label(), finding.summary);
            let _ = writeln!(out, "    {}\n", finding.advice);
        }

        if self.healthy() {
            if self.runs == 0 {
                out.push_str("No verdict: there is no history to judge this factory by.\n");
            } else {
                out.push_str("Nothing here would fail an unattended run.\n");
            }
        } else {
            out.push_str("This factory would not pass an unattended soak as it stands.\n");
        }

        out
    }
}

/// Examines a factory's recorded state.
///
/// `window` bounds how far back to look. A soak checks the whole soak; a daily glance checks a
/// day.
///
/// # Errors
///
/// Returns an error when the state directory cannot be read. A factory whose history cannot be
/// read is itself the finding, and one worth stopping on.
pub fn check(config: &Config, root: &Path, window: Window) -> Result<Report, String> {
    let history =
        History::open(root.join(".layover").join("history")).map_err(|error| error.to_string())?;
    let journal =
        Journal::open(root.join(".layover").join("journal")).map_err(|error| error.to_string())?;

    let span = history.resolve(window);
    let runs = history
        .runs(&span, &RunFilter::default())
        .map_err(|error| error.to_string())?;

    let mut report = Report {
        runs: runs.len(),
        hours: runs
            .iter()
            .map(|record| record.started_at)
            .min()
            .map_or(0, |first| {
                (Timestamp::now().as_second() - first.as_second()) / 3_600
            }),
        findings: Vec::new(),
    };

    stalled_chains(&journal, &span, &mut report);
    unreported_costs(&runs, &mut report);
    interrupted_runs(&runs, &mut report);
    open_help(&journal, &span, &mut report);
    stranded_layovers(&journal, config, &mut report);
    ground_stop(root, &mut report);

    // Only meaningful against history. With an empty window "this schedule never fired" is not a
    // finding about the schedule, it is the same fact as "nothing ran", and reporting it once per
    // pipeline turns a factory nobody has started yet into a page of warnings.
    if runs.is_empty() {
        report.findings.push(Finding {
            severity: Severity::Note,
            summary: format!("no runs recorded in the {} window", window.label()),
            advice: "Nothing here is a verdict on the factory — there is no history to judge. \
                     Either it has not been started, or `--window` is narrower than the gap \
                     between runs."
                .to_owned(),
        });
    } else {
        idle_schedules(config, &runs, &mut report);
    }

    report
        .findings
        .sort_by_key(|finding| std::cmp::Reverse(finding.severity));
    Ok(report)
}

/// Chains that stopped with nothing left to run them.
fn stalled_chains(journal: &Journal, span: &Span, report: &mut Report) {
    let Ok(stalls) = journal.stalls(span) else {
        return;
    };

    if stalls.is_empty() {
        return;
    }

    let stranded: usize = stalls.iter().map(|stall| stall.stranded).sum();

    report.findings.push(Finding {
        severity: Severity::Fault,
        summary: format!(
            "{} chain(s) stalled, stranding {stranded} flight(s)",
            stalls.len()
        ),
        advice: "Work somebody asked for that will not happen. Each stall names the agent that \
                 never woke and what it was waiting for; a join whose upstream cannot be reached \
                 is usually a route map that disagrees with the prompts."
            .to_owned(),
    });
}

/// Runs whose cost is a floor rather than a figure.
fn unreported_costs(runs: &[layover_core::run::RunRecord], report: &mut Report) {
    let silent = runs
        .iter()
        .filter(|record| record.source != CostSource::Reported)
        .count();

    if silent == 0 {
        return;
    }

    // A proportion rather than a count: one silent run in a thousand is a runner quirk, and half
    // of them is a factory whose spending nobody can actually see.
    let share = silent * 100 / runs.len().max(1);

    report.findings.push(Finding {
        severity: if share >= 25 {
            Severity::Warning
        } else {
            Severity::Note
        },
        summary: format!(
            "{silent} of {} run(s) reported no cost ({share}%)",
            runs.len()
        ),
        advice: "Every total that includes these is a floor, not a figure. Fuel is the rail that \
                 depends on runners reporting honestly; the run cap is the one that does not, and \
                 it is what is actually holding."
            .to_owned(),
    });
}

/// Runs the supervisor had to stop, or never saw finish.
fn interrupted_runs(runs: &[layover_core::run::RunRecord], report: &mut Report) {
    let timed_out = runs
        .iter()
        .filter(|record| record.outcome == Outcome::TimedOut)
        .count();

    if timed_out == 0 {
        return;
    }

    report.findings.push(Finding {
        severity: if timed_out > runs.len() / 10 {
            Severity::Warning
        } else {
            Severity::Note
        },
        summary: format!("{timed_out} run(s) outlived `timeout_sec` and were killed"),
        advice: "A timeout is the supervisor doing its job, but a wedged agent repeats \
                 identically — the same run will wedge again next time. Raise `timeout_sec` only \
                 if the work genuinely takes that long."
            .to_owned(),
    });
}

/// Agents that asked for a person and did not get one.
fn open_help(journal: &Journal, span: &Span, report: &mut Report) {
    let filter = HelpFilter {
        open_only: true,
        ..HelpFilter::default()
    };

    let Ok(open) = journal.help(span, &filter) else {
        return;
    };

    if open.is_empty() {
        return;
    }

    let fatal = open.iter().filter(|request| request.fatal).count();

    report.findings.push(Finding {
        severity: if fatal > 0 {
            Severity::Warning
        } else {
            Severity::Note
        },
        summary: format!(
            "{} open help request(s), {fatal} of which stopped the work",
            open.len()
        ),
        advice: "This is the channel that reaches a person, and nothing else does. An unattended \
                 factory that has been asking for two days is one that stopped making progress on \
                 the first."
            .to_owned(),
    });
}

/// Work set down that nothing will pick up.
fn stranded_layovers(journal: &Journal, config: &Config, report: &mut Report) {
    let Ok(all) = journal.layovers() else {
        return;
    };

    let expired = all
        .iter()
        .filter(|layover| layover.standing == layover_core::layover::Standing::Expired)
        .count();

    if expired > 0 {
        report.findings.push(Finding {
            severity: Severity::Warning,
            summary: format!("{expired} layover(s) gave up after being checked too many times"),
            advice: "Each was waiting for something that never happened. That is either a \
                     follow-up nobody answered or an agent waiting on the wrong signal."
                .to_owned(),
        });
    }

    let waiting = all
        .iter()
        .filter(|layover| layover.standing.is_pending())
        .count();

    if waiting == 0 {
        return;
    }

    // The factory definition is right here, so this is a fact rather than a caveat: a layover is
    // only ever collected by a pipeline that declares `resumes`, and with none declared every one
    // of these is work an agent deliberately set down and nothing will ever pick up again.
    let collectable = config.pipelines.values().any(|pipeline| pipeline.resumes);

    report.findings.push(if collectable {
        Finding {
            severity: Severity::Note,
            summary: format!("{waiting} layover(s) waiting to be picked up"),
            advice: "Expected: a resuming pipeline collects them when their time comes.".to_owned(),
        }
    } else {
        Finding {
            severity: Severity::Fault,
            summary: format!(
                "{waiting} layover(s) waiting, and no pipeline declares `resumes = true`"
            ),
            advice: "Nothing will ever collect them. An agent set this work down meaning to come \
                     back to it, and the factory has no way back. Add `resumes = true` to the \
                     pipeline that should pick it up."
                .to_owned(),
        }
    });
}

/// Whether everything is halted.
fn ground_stop(root: &Path, report: &mut Report) {
    if !root.join(".layover").join("ground-stop").exists() {
        return;
    }

    report.findings.push(Finding {
        severity: Severity::Warning,
        summary: "a Ground Stop is engaged".to_owned(),
        advice: "Nothing will start while it is. If this was not deliberate, release it from the \
                 dashboard or delete `.layover/ground-stop`."
            .to_owned(),
    });
}

/// Scheduled pipelines that have never actually run.
fn idle_schedules(config: &Config, runs: &[layover_core::run::RunRecord], report: &mut Report) {
    for (name, _) in config.scheduled_pipelines() {
        let ran = runs
            .iter()
            .any(|record| record.pipeline.as_ref() == Some(name));

        if !ran {
            report.findings.push(Finding {
                severity: Severity::Warning,
                summary: format!("scheduled pipeline `{name}` has not run in this window"),
                advice: "A schedule that never fires looks exactly like one that is working. \
                         Check that `layover serve` is running without `--watch-only`, and that \
                         the interval is shorter than the window being examined."
                    .to_owned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_with_nothing_in_it_says_so_plainly() {
        let report = Report {
            findings: Vec::new(),
            runs: 40,
            hours: 48,
        };

        assert!(report.healthy());
        assert!(
            report.render().contains("Nothing to report"),
            "{}",
            report.render()
        );
    }

    #[test]
    fn a_fault_means_the_factory_would_not_pass_a_soak() {
        let report = Report {
            findings: vec![Finding {
                severity: Severity::Fault,
                summary: "1 chain stalled".to_owned(),
                advice: "look".to_owned(),
            }],
            runs: 10,
            hours: 48,
        };

        assert!(!report.healthy());
        assert!(
            report.render().contains("would not pass"),
            "{}",
            report.render()
        );
    }

    #[test]
    fn a_note_on_its_own_does_not_fail_anything() {
        // Notes exist to be read, not to block. A check that failed on every curiosity would be
        // one people stopped running.
        let report = Report {
            findings: vec![Finding {
                severity: Severity::Note,
                summary: "2 runs reported no cost".to_owned(),
                advice: "look".to_owned(),
            }],
            runs: 100,
            hours: 48,
        };

        assert!(report.healthy());
        assert!(
            report.render().contains("Nothing here would fail"),
            "{}",
            report.render()
        );
    }

    #[test]
    fn an_empty_window_gives_no_verdict_rather_than_a_pass() {
        // A factory nobody has started has not passed anything. Saying "nothing would fail" of
        // an empty history reads as a clean bill of health for a factory that has never run.
        let report = Report {
            findings: vec![Finding {
                severity: Severity::Note,
                summary: "no runs recorded".to_owned(),
                advice: String::new(),
            }],
            runs: 0,
            hours: 0,
        };

        assert!(report.healthy());
        assert!(
            report.render().contains("No verdict"),
            "{}",
            report.render()
        );
    }

    #[test]
    fn a_waiting_layover_is_a_fault_when_no_pipeline_can_ever_collect_it() {
        // Found on the first real end-to-end run: an agent booked a layover in a factory with no
        // resuming pipeline. Every run said "succeeded", the chain looked finished, and the work
        // the agent meant to come back to was simply gone. Hedging with "normal, as long as a
        // pipeline declares `resumes`" left the reader to check the thing the checker could see.
        let dir = std::env::temp_dir().join(format!("layover-doctor-lay-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = Journal::open(&dir).expect("opens");

        journal
            .book(layover_core::Layover::book(
                layover_core::agent::AgentName::new("analyst"),
                layover_core::flight::ItineraryId::generate(),
                "a review nobody has done yet",
                layover_core::handover::Handover::dispatch(Vec::new()),
                Timestamp::now(),
                Timestamp::now(),
                3,
            ))
            .expect("books");

        let with_resuming: Config = toml::from_str(
            r#"
[agents.analyst]
prompt = "go"

[pipelines.follow_up]
entry = "analyst"
resumes = true
"#,
        )
        .expect("parses");

        let without: Config = toml::from_str(
            r#"
[agents.analyst]
prompt = "go"

[pipelines.once]
entry = "analyst"
"#,
        )
        .expect("parses");

        let mut collectable = Report::default();
        stranded_layovers(&journal, &with_resuming, &mut collectable);
        assert_eq!(collectable.findings[0].severity, Severity::Note);

        let mut stranded = Report::default();
        stranded_layovers(&journal, &without, &mut stranded);
        assert_eq!(stranded.findings[0].severity, Severity::Fault);
        assert!(
            stranded.findings[0].summary.contains("resumes"),
            "{}",
            stranded.findings[0].summary
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn findings_are_ordered_worst_first() {
        // Somebody reading this at the end of two days reads the top of it.
        let mut report = Report {
            findings: vec![
                Finding {
                    severity: Severity::Note,
                    summary: "a note".to_owned(),
                    advice: String::new(),
                },
                Finding {
                    severity: Severity::Fault,
                    summary: "a fault".to_owned(),
                    advice: String::new(),
                },
            ],
            runs: 1,
            hours: 1,
        };

        report
            .findings
            .sort_by_key(|finding| std::cmp::Reverse(finding.severity));

        assert_eq!(report.findings[0].severity, Severity::Fault);
    }

    #[test]
    fn silent_costs_are_a_note_when_rare_and_a_warning_when_common() {
        // One silent run in a thousand is a runner quirk; half of them is a factory whose
        // spending nobody can see.
        let run = |source: CostSource| layover_core::run::RunRecord {
            run: layover_core::RunId::generate(),
            itinerary: layover_core::flight::ItineraryId::generate(),
            agent: layover_core::agent::AgentName::new("worker"),
            pipeline: None,
            model: None,
            outcome: Outcome::Succeeded,
            started_at: Timestamp::now(),
            finished_at: Some(Timestamp::now()),
            usd: 0.0,
            source,
            usage: layover_core::cost::TokenUsage::default(),
            detail: None,
            blocked_on: None,
            pid: None,
        };

        let mut rare = Report::default();
        let mut runs: Vec<_> = (0..20).map(|_| run(CostSource::Reported)).collect();
        runs.push(run(CostSource::Unreported));
        unreported_costs(&runs, &mut rare);
        assert_eq!(rare.findings[0].severity, Severity::Note);

        let mut common = Report::default();
        let half: Vec<_> = (0..10)
            .map(|index| {
                run(if index % 2 == 0 {
                    CostSource::Reported
                } else {
                    CostSource::Unreported
                })
            })
            .collect();
        unreported_costs(&half, &mut common);
        assert_eq!(common.findings[0].severity, Severity::Warning);
    }
}
