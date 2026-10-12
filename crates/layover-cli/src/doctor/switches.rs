//! What `layover doctor` says about workflows a person switched off.
//!
//! A note rather than a warning: a switch is flipped on purpose, and the dashboard says it is off
//! wherever the workflow is shown. Said here because a doctor run is what somebody reads after a
//! quiet weekend, and "the review sweep never ran" has a different answer when it was switched off.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use layover_core::config::Config;
use layover_core::pipeline::PipelineName;
use layover_store::Journal;

use super::{Finding, Report, Severity};

/// Reports every workflow that is switched off, and returns them so other checks can leave them be.
pub(super) fn switched_off(
    config: &Config,
    journal: &Journal,
    report: &mut Report,
) -> BTreeSet<PipelineName> {
    let off = journal.switched_off().unwrap_or_default();
    if off.is_empty() {
        return BTreeSet::new();
    }

    let (declared, gone): (Vec<_>, Vec<_>) = off
        .iter()
        .partition(|off| config.pipelines.contains_key(&off.pipeline));

    let mut advice = "Their schedules do not fire, triggers are refused, a resuming one collects \
                      nothing and a trigger already queued waits, until they are switched back on \
                      from the dashboard. Chains already under way finish."
        .to_owned();
    if !gone.is_empty() {
        let names: Vec<String> = gone
            .iter()
            .map(|off| format!("`{}`", off.pipeline))
            .collect();
        let (who, lines) = if names.len() == 1 {
            (format!("{} is", names[0]), "its line does")
        } else {
            (format!("{} are", names.join(", ")), "their lines do")
        };
        let _ = write!(
            advice,
            " {who} no longer declared, so {lines} in `.layover/journal/switched-off.jsonl` \
             nothing."
        );
    }

    let named: Vec<String> = declared
        .iter()
        .map(|off| match off.since {
            Some(since) => format!(
                "`{}` (since {})",
                off.pipeline,
                since.round(jiff::Unit::Minute).unwrap_or(since)
            ),
            None => format!("`{}`", off.pipeline),
        })
        .collect();
    report.findings.push(Finding {
        severity: Severity::Note,
        summary: if named.is_empty() {
            "a workflow that no longer exists is switched off".to_owned()
        } else {
            format!(
                "{} workflow(s) switched off: {}",
                named.len(),
                named.join(", ")
            )
        },
        advice,
    });

    off.into_iter().map(|off| off.pipeline).collect()
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use super::*;

    #[test]
    fn a_switched_off_workflow_is_a_note_and_a_forgotten_one_is_named() {
        let dir = std::env::temp_dir().join(format!("layover-doctor-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = Journal::open(&dir).expect("opens");
        let config: Config = toml::from_str(
            r#"
[agents.eagle]
prompt = "review"

[pipelines.sweep]
entry = "eagle"
trigger = { every = "2h" }
"#,
        )
        .expect("parses");
        for name in ["sweep", "renamed-long-ago"] {
            journal
                .switch_off(&PipelineName::new(name), Timestamp::now())
                .expect("switches off");
        }

        let mut report = Report::default();
        let off = switched_off(&config, &journal, &mut report);

        assert_eq!(off.len(), 2);
        assert_eq!(report.findings.len(), 1);
        let finding = &report.findings[0];
        assert_eq!(finding.severity, Severity::Note);
        assert!(finding.summary.contains("`sweep`"), "{}", finding.summary);
        assert!(
            finding
                .advice
                .contains("`renamed-long-ago` is no longer declared"),
            "{}",
            finding.advice
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
