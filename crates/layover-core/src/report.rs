//! What an agent writes about its own run.
//!
//! A run leaves three kinds of trace and they answer different questions. The [`RunRecord`] says
//! *that* it happened — agent, outcome, duration, cost — and is what a list is built from. A
//! [`HelpRequest`] says what got in the way. This says what the agent actually *did*, in its own
//! words, and it is the only one of the three a person reads for content rather than for status.
//!
//! [`RunRecord`]: crate::run::RunRecord
//! [`HelpRequest`]: crate::help::HelpRequest
//!
//! # Why the agent writes it rather than the Tower capturing output
//!
//! A transcript is not a report. It contains everything the agent thought, including the three
//! approaches it abandoned, and reading one to find out what happened is slower than doing the
//! work again. The Tower could persist transcripts and separately ask for a summary, but asking
//! an agent to state its own conclusion has a second effect worth more than the storage: an agent
//! that must write down what it concluded is an agent that has to decide what it concluded.
//!
//! # Why it is capped
//!
//! Reports accumulate at one per run and are read in lists. An unbounded body is a slow page and
//! eventually a large disk, and an agent asked for "a report" with no limit will produce its
//! transcript. The cap is generous enough for real prose and small enough that the intent is
//! obvious.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::agent::AgentName;
use crate::flight::{ItineraryId, RunId};

/// Longest a headline may be.
///
/// One line in a list. Anything longer is a body pretending to be a headline.
pub const MAX_HEADLINE: usize = 160;

/// Longest a body may be, in characters.
///
/// Roughly two thousand words. Past that an agent is pasting rather than reporting.
pub const MAX_BODY: usize = 12_000;

/// Most artefacts one report may name.
pub const MAX_ARTIFACTS: usize = 32;

/// What one run concluded.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Report {
    /// The run this describes.
    pub run: RunId,
    /// Which agent wrote it.
    pub agent: AgentName,
    /// The chain it belonged to.
    pub itinerary: ItineraryId,
    /// One line, for a list. What happened, not what was attempted.
    pub headline: String,
    /// The report itself, as the agent wrote it.
    pub body: String,
    /// What it produced or changed: paths, branch names, pull request identifiers.
    ///
    /// Separate from the body so a reader can find the output without reading the prose, and so
    /// a later run can be told what an earlier one left behind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
    /// When it was written.
    pub at: Timestamp,
}

impl Report {
    /// Records a report, trimming anything past the caps.
    ///
    /// Truncates rather than rejecting. A report is the only account of a run that has already
    /// happened and already cost money; throwing it away because it was too long would lose the
    /// thing entirely to punish a formatting mistake.
    #[must_use]
    pub fn new(
        run: RunId,
        agent: AgentName,
        itinerary: ItineraryId,
        headline: impl Into<String>,
        body: impl Into<String>,
        at: Timestamp,
    ) -> Self {
        Self {
            run,
            agent,
            itinerary,
            headline: clamp(&headline.into(), MAX_HEADLINE),
            body: clamp(&body.into(), MAX_BODY),
            artifacts: Vec::new(),
            at,
        }
    }

    /// Names what the run produced.
    #[must_use]
    pub fn producing(mut self, artifacts: Vec<String>) -> Self {
        self.artifacts = artifacts.into_iter().take(MAX_ARTIFACTS).collect();
        self
    }

    /// Returns `true` when the agent had something to say beyond the headline.
    #[must_use]
    pub fn has_body(&self) -> bool {
        !self.body.trim().is_empty()
    }

    /// Returns `true` when anything was trimmed to fit.
    ///
    /// Worth surfacing: a reader who can see the report was cut knows to look at the transcript
    /// rather than assuming the agent stopped there.
    #[must_use]
    pub fn was_trimmed(&self) -> bool {
        self.headline.ends_with('…') || self.body.ends_with('…')
    }
}

/// Shortens text to `limit` characters, marking it when it had to.
fn clamp(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_owned();
    }

    let mut kept: String = trimmed.chars().take(limit.saturating_sub(1)).collect();
    kept.push('…');
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> Timestamp {
        "2026-09-17T10:00:00Z".parse().expect("valid timestamp")
    }

    fn report(headline: &str, body: &str) -> Report {
        Report::new(
            RunId::generate(),
            "developer".into(),
            ItineraryId::generate(),
            headline,
            body,
            at(),
        )
    }

    #[test]
    fn a_report_keeps_what_the_agent_wrote() {
        let written = report(
            "Fixed the proxy option mismatch",
            "The three assertions were incompatible; changed the factory to take the resolved \
             options rather than rebuilding them.",
        );

        assert_eq!(written.headline, "Fixed the proxy option mismatch");
        assert!(written.has_body());
        assert!(!written.was_trimmed());
    }

    #[test]
    fn an_overlong_report_is_trimmed_rather_than_refused() {
        // A report is the only account of a run that has already happened and already cost
        // money. Throwing it away to punish a formatting mistake loses the thing entirely.
        let written = report(&"x".repeat(MAX_HEADLINE + 50), &"y".repeat(MAX_BODY + 500));

        assert_eq!(written.headline.chars().count(), MAX_HEADLINE);
        assert_eq!(written.body.chars().count(), MAX_BODY);
        assert!(written.was_trimmed());
    }

    #[test]
    fn a_reader_can_tell_a_report_was_cut() {
        // Otherwise it looks like the agent simply stopped there, and nobody goes looking for
        // the rest.
        assert!(report(&"x".repeat(MAX_HEADLINE + 1), "short").was_trimmed());
        assert!(!report("short", "short").was_trimmed());
    }

    #[test]
    fn artifacts_are_capped_but_the_report_survives() {
        let many: Vec<String> = (0..MAX_ARTIFACTS + 10)
            .map(|n| format!("file{n}.rs"))
            .collect();
        let written = report("did a lot", "body").producing(many);

        assert_eq!(written.artifacts.len(), MAX_ARTIFACTS);
        assert!(written.has_body());
    }

    #[test]
    fn a_report_with_nothing_in_the_body_is_recognisable() {
        // A headline alone is a legitimate report -- "nothing had changed" is a complete account
        // of a follow-up run -- but a reader should not be offered an empty panel to open.
        let written = report("Nothing had changed on the pull request", "   ");

        assert!(!written.has_body());
    }

    #[test]
    fn a_report_serialises_to_one_readable_line() {
        let line = serde_json::to_string(&report("Fixed it", "Details here")).expect("serialises");

        assert!(!line.contains('\n'));
        assert!(line.contains(r#""headline":"Fixed it""#), "{line}");
        assert!(line.contains(r#""at":"2026-09-17T10:00:00Z""#), "{line}");
        assert!(!line.contains("artifacts"), "absent, not empty: {line}");
    }

    #[test]
    fn a_report_round_trips() {
        let written = report("Fixed it", "Details").producing(vec!["branch/fix-1".to_owned()]);
        let line = serde_json::to_string(&written).expect("serialises");

        assert_eq!(
            serde_json::from_str::<Report>(&line).expect("deserialises"),
            written
        );
    }
}
