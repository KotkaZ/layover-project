//! What a run was, once it is over.
//!
//! [`crate::cost::RunCost`] records what a run *spent*. This records what it *did*: which agent,
//! in which chain, started by which pipeline, and how it ended. The dashboard needs both, and
//! they are separate types because cost is a rail that must work even when history is turned off.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::agent::AgentName;
use crate::cost::{CostSource, TokenUsage};
use crate::flight::{ItineraryId, RunId};
use crate::pipeline::PipelineName;

/// How a run ended.
///
/// `Running` is here rather than in a separate "live" type so that one query answers both "what
/// is happening now" and "what happened last week". A dashboard that had to stitch two sources
/// together would show them disagreeing during the moment a run finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Still going.
    Running,
    /// Exited successfully.
    Succeeded,
    /// Exited non-zero, or the runner reported failure.
    Failed,
    /// Hit `timeout_sec` and was killed.
    TimedOut,
    /// Stopped because a rail refused it: Hops, Fuel, the run cap or the Reserve.
    Halted,
    /// Was alive when the Tower went away. See the recovery model.
    Interrupted,
}

impl Outcome {
    /// Returns `true` when the run is still going.
    #[must_use]
    pub fn is_live(self) -> bool {
        matches!(self, Self::Running)
    }

    /// Returns `true` when the run finished the way it was supposed to.
    #[must_use]
    pub fn is_success(self) -> bool {
        matches!(self, Self::Succeeded)
    }

    /// Returns `true` when the run ended badly enough to be worth a human's attention.
    ///
    /// [`Outcome::Halted`] is deliberately excluded. A rail stopping work is the system doing its
    /// job, and colouring it like a crash would train people to ignore the colour.
    #[must_use]
    pub fn is_failure(self) -> bool {
        matches!(self, Self::Failed | Self::TimedOut | Self::Interrupted)
    }

    /// The identifier used in query strings and JSON.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Halted => "halted",
            Self::Interrupted => "interrupted",
        }
    }

    /// Parses a slug, as used in a query string.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        [
            Self::Running,
            Self::Succeeded,
            Self::Failed,
            Self::TimedOut,
            Self::Halted,
            Self::Interrupted,
        ]
        .into_iter()
        .find(|outcome| outcome.slug() == slug)
    }
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.slug())
    }
}

/// One supervised CLI execution, as recorded in history.
///
/// Serialised one per line as JSON. Field names are the wire format: renaming one silently
/// orphans every record already on disk, so treat them as an API.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct RunRecord {
    /// The run.
    pub run: RunId,
    /// The chain it belonged to.
    pub itinerary: ItineraryId,
    /// Which agent was run.
    pub agent: AgentName,
    /// The pipeline that started the chain, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<PipelineName>,
    /// Which model, when the runner said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How it ended.
    pub outcome: Outcome,
    /// When it started.
    pub started_at: Timestamp,
    /// When it ended, or `None` while it is still going.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<Timestamp>,
    /// Cost in US dollars.
    #[serde(default)]
    pub usd: f64,
    /// Where `usd` came from.
    pub source: CostSource,
    /// Tokens consumed, as far as they are known.
    #[serde(default)]
    pub usage: TokenUsage,
    /// Why it ended, for the outcomes where that is not obvious.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The operating system process id, while the run is live.
    ///
    /// Recorded so that a Tower coming back from a restart can *check* whether the process is
    /// still there rather than assume. A child routinely outlives the parent that spawned it on
    /// Windows, and recovering beside a process that never stopped duplicates its work — see
    /// [`crate::handover::ChildState`].
    ///
    /// A recycled process id can make a dead run look alive, which fails towards refusing to
    /// recover. That is the safe direction: stalled work is visible, duplicated work is not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

impl RunRecord {
    /// Records a run that has just started.
    #[must_use]
    pub fn started(
        run: RunId,
        itinerary: ItineraryId,
        agent: AgentName,
        started_at: Timestamp,
    ) -> Self {
        Self {
            run,
            itinerary,
            agent,
            pipeline: None,
            model: None,
            outcome: Outcome::Running,
            started_at,
            finished_at: None,
            usd: 0.0,
            source: CostSource::Unreported,
            usage: TokenUsage::default(),
            detail: None,
            pid: None,
        }
    }

    /// Attributes the run to the pipeline that started its chain.
    #[must_use]
    pub fn from_pipeline(mut self, pipeline: PipelineName) -> Self {
        self.pipeline = Some(pipeline);
        self
    }

    /// Notes which model the runner used.
    #[must_use]
    pub fn using_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Closes the run out with an outcome and a finishing time.
    #[must_use]
    pub fn finished(mut self, outcome: Outcome, at: Timestamp) -> Self {
        self.outcome = outcome;
        self.finished_at = Some(at);
        self
    }

    /// Attaches what the run cost.
    #[must_use]
    pub fn costing(mut self, usd: f64, source: CostSource, usage: TokenUsage) -> Self {
        self.usd = usd;
        self.source = source;
        self.usage = usage;
        self
    }

    /// Explains an outcome that is not self-evident.
    #[must_use]
    pub fn because(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Notes the process id, so a later Tower can check whether it is still running.
    #[must_use]
    pub fn with_pid(mut self, pid: u32) -> Self {
        self.pid = Some(pid);
        self
    }

    /// How long the run took, in whole seconds, or `None` while it is still going.
    ///
    /// Returns `None` rather than a negative number if the clock went backwards between the two
    /// readings, which NTP correction can do: a negative duration on a dashboard is worse than an
    /// absent one, because somebody will average it.
    #[must_use]
    pub fn duration_secs(&self) -> Option<i64> {
        let finished = self.finished_at?;
        let seconds = finished.as_second() - self.started_at.as_second();
        (seconds >= 0).then_some(seconds)
    }

    /// The instant this record should be filed under.
    ///
    /// Runs are filed by when they *finished*, matching the cost ledger, so that a total for a
    /// period covers the runs whose money landed in it. A long run started before a window and
    /// finished inside it belongs to the window it was paid for.
    #[must_use]
    pub fn filed_at(&self) -> Timestamp {
        self.finished_at.unwrap_or(self.started_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rfc3339: &str) -> Timestamp {
        rfc3339.parse().expect("valid timestamp")
    }

    fn record() -> RunRecord {
        RunRecord::started(
            RunId::generate(),
            ItineraryId::generate(),
            "developer".into(),
            at("2026-09-16T10:00:00Z"),
        )
    }

    #[test]
    fn a_finished_run_reports_how_long_it_took() {
        let done = record().finished(Outcome::Succeeded, at("2026-09-16T10:04:30Z"));

        assert_eq!(done.duration_secs(), Some(270));
        assert!(done.outcome.is_success());
        assert!(!done.outcome.is_live());
    }

    #[test]
    fn a_running_run_has_no_duration_yet() {
        let live = record();

        assert_eq!(live.duration_secs(), None);
        assert!(live.outcome.is_live());
        assert_eq!(live.filed_at(), live.started_at);
    }

    #[test]
    fn a_backwards_clock_produces_no_duration_rather_than_a_negative_one() {
        // NTP correction can move the clock between the two readings. A negative duration is
        // worse than a missing one, because it will end up in an average.
        let impossible = record().finished(Outcome::Succeeded, at("2026-09-16T09:59:00Z"));

        assert_eq!(impossible.duration_secs(), None);
    }

    #[test]
    fn runs_are_filed_by_when_they_finished() {
        // Matching the cost ledger, so that a period's total covers the runs whose money landed
        // in it rather than the ones that happened to begin in it.
        let done = record().finished(Outcome::Succeeded, at("2026-09-16T10:04:30Z"));

        assert_eq!(done.filed_at(), at("2026-09-16T10:04:30Z"));
    }

    #[test]
    fn a_rail_stopping_work_is_not_a_failure() {
        // Hops, Fuel and the Reserve doing their job is the system working. Colouring it like a
        // crash teaches people to ignore the colour, which is how a real crash gets missed.
        assert!(!Outcome::Halted.is_failure());
        assert!(Outcome::Failed.is_failure());
        assert!(Outcome::TimedOut.is_failure());
        assert!(Outcome::Interrupted.is_failure());
    }

    #[test]
    fn outcome_slugs_round_trip() {
        for outcome in [
            Outcome::Running,
            Outcome::Succeeded,
            Outcome::Failed,
            Outcome::TimedOut,
            Outcome::Halted,
            Outcome::Interrupted,
        ] {
            assert_eq!(Outcome::from_slug(outcome.slug()), Some(outcome));
        }

        assert_eq!(Outcome::from_slug("exploded"), None);
    }

    #[test]
    fn a_record_serialises_to_one_readable_line() {
        // The file is meant to be openable in a text editor, which is most of why it is JSON
        // Lines rather than a database. Timestamps must read as dates, not as epoch structs.
        let done = record()
            .from_pipeline("triage".into())
            .using_model("claude-opus-5")
            .finished(Outcome::Succeeded, at("2026-09-16T10:04:30Z"))
            .costing(1.25, CostSource::Reported, TokenUsage::default());

        let line = serde_json::to_string(&done).expect("serialises");

        assert!(
            !line.contains('\n'),
            "a record must occupy exactly one line"
        );
        assert!(
            line.contains(r#""started_at":"2026-09-16T10:00:00Z""#),
            "{line}"
        );
        assert!(line.contains(r#""outcome":"succeeded""#), "{line}");
        assert!(line.contains(r#""pipeline":"triage""#), "{line}");
    }

    #[test]
    fn absent_optional_fields_are_left_out_rather_than_written_as_null() {
        let line = serde_json::to_string(&record()).expect("serialises");

        assert!(!line.contains("null"), "{line}");
        assert!(!line.contains("finished_at"), "{line}");
    }

    #[test]
    fn a_live_run_records_the_process_it_is_waiting_on() {
        // Without this a Tower coming back from a restart has nothing to check, and must either
        // assume the child died -- which duplicates work when it did not -- or never recover.
        let spawned = record().with_pid(4242);

        let line = serde_json::to_string(&spawned).expect("serialises");
        assert!(line.contains(r#""pid":4242"#), "{line}");

        let closed = spawned.finished(Outcome::Succeeded, at("2026-09-16T10:01:00Z"));
        assert_eq!(closed.pid, Some(4242), "the id survives the run ending");
    }

    #[test]
    fn a_record_round_trips_through_its_wire_format() {
        let done = record()
            .from_pipeline("triage".into())
            .finished(Outcome::Halted, at("2026-09-16T10:01:00Z"))
            .because("out of Fuel");

        let line = serde_json::to_string(&done).expect("serialises");
        let back: RunRecord = serde_json::from_str(&line).expect("deserialises");

        assert_eq!(back, done);
        assert_eq!(back.detail.as_deref(), Some("out of Fuel"));
    }
}
