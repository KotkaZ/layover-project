//! The history directory: appending records, reading windows and enforcing retention.

use std::fs;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use layover_core::agent::AgentName;
use layover_core::cost::{Ledger, RunCost, Span, Window};
use layover_core::pipeline::PipelineName;
use layover_core::run::{Outcome, RunRecord};

use crate::segment::{append_line, prune_segments, read_segment, segment_for, segments_covering};

/// Filename prefix for run segments.
const RUNS: &str = "runs";

/// Something went wrong reaching the history directory.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The directory could not be created or written to.
    #[error("history at {path}: {source}")]
    Io {
        /// Where the trouble was.
        path: PathBuf,
        /// What the filesystem said.
        #[source]
        source: std::io::Error,
    },
    /// A record could not be turned into a line.
    #[error("could not serialise a run record: {0}")]
    Serialise(#[from] serde_json::Error),
}

impl StoreError {
    pub(crate) fn at(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        |source| Self::Io { path, source }
    }
}

/// Which runs to return.
///
/// Every field is a narrowing; an empty filter matches everything in the window.
#[derive(Debug, Clone, Default)]
pub struct RunFilter {
    /// Only runs of this agent.
    pub agent: Option<AgentName>,
    /// Only runs started by this pipeline.
    pub pipeline: Option<PipelineName>,
    /// Only runs that ended this way.
    pub outcome: Option<Outcome>,
    /// At most this many, most recent first. `None` means all of them.
    pub limit: Option<usize>,
}

impl RunFilter {
    /// Returns `true` when `record` passes every narrowing.
    fn matches(&self, record: &RunRecord) -> bool {
        self.agent.as_ref().is_none_or(|a| *a == record.agent)
            && self
                .pipeline
                .as_ref()
                .is_none_or(|p| record.pipeline.as_ref() == Some(p))
            && self.outcome.is_none_or(|o| o == record.outcome)
    }
}

/// An append-only history of runs, kept in a directory of day-segmented JSON Lines files.
#[derive(Debug, Clone)]
pub struct History {
    root: PathBuf,
    zone: TimeZone,
}

impl History {
    /// Opens — and creates, if needed — a history directory.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the directory cannot be created.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(StoreError::at(&root))?;
        Ok(Self {
            root,
            zone: TimeZone::system(),
        })
    }

    /// Reckons calendar windows in `zone` rather than the system's.
    ///
    /// Exists so tests do not depend on where the machine is standing, and so a factory whose
    /// operator is not in the machine's zone can report in theirs.
    #[must_use]
    pub fn in_zone(mut self, zone: TimeZone) -> Self {
        self.zone = zone;
        self
    }

    /// Where the history lives.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The zone calendar windows are reckoned in.
    #[must_use]
    pub fn zone(&self) -> &TimeZone {
        &self.zone
    }

    /// Resolves a window against the current moment in this history's zone.
    ///
    /// # Errors
    ///
    /// Never returns an error today; the signature leaves room for a zone that cannot be
    /// resolved without forcing every caller to change later.
    #[must_use]
    pub fn resolve(&self, window: Window) -> Span {
        window.resolve(&Timestamp::now().to_zoned(self.zone.clone()))
    }

    /// Appends a record.
    ///
    /// Opened, written and closed per record. That is slower than holding the handle open and it
    /// is the right trade: a factory writes a handful of records a minute, and a file that is
    /// never held open cannot be left truncated by a process that dies mid-run — which, given
    /// the whole point of recording interruptions, is not a hypothetical.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the line cannot be written, or [`StoreError::Serialise`] if
    /// the record cannot be encoded.
    pub fn append(&self, record: &RunRecord) -> Result<(), StoreError> {
        append_line(
            &self.segment_for(record.filed_at()),
            &serde_json::to_string(record)?,
        )
    }

    /// Reads the runs that fall inside `span`, most recent first.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if a segment exists but cannot be read. A segment that is
    /// simply absent is not an error — it means nothing happened that day.
    pub fn runs(&self, span: &Span, filter: &RunFilter) -> Result<Vec<RunRecord>, StoreError> {
        let mut found = Vec::new();

        for path in segments_covering(&self.root, RUNS, span)? {
            for record in read_segment::<RunRecord>(&path)? {
                if span.contains(record.filed_at()) && filter.matches(&record) {
                    found.push(record);
                }
            }
        }

        found.sort_by_key(|record| std::cmp::Reverse(record.filed_at()));
        if let Some(limit) = filter.limit {
            found.truncate(limit);
        }
        Ok(found)
    }

    /// Builds a cost ledger from the runs in `span`.
    ///
    /// Runs whose cost was never reported still contribute, as [`layover_core::cost::CostSource`]
    /// holes rather than as zeroes, which is what keeps a total honest about what it does not
    /// know.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if a segment cannot be read.
    pub fn ledger(&self, span: &Span) -> Result<Ledger, StoreError> {
        let mut ledger = Ledger::new();

        for record in self.runs(span, &RunFilter::default())? {
            // A run still going has not been billed yet; counting it would make the total move
            // backwards when the real figure arrives.
            if record.outcome.is_live() {
                continue;
            }
            let at = record.filed_at();
            ledger.record(RunCost {
                run: record.run,
                itinerary: record.itinerary,
                agent: record.agent,
                model: record.model,
                usage: record.usage,
                usd: record.usd,
                source: record.source,
                at,
            });
        }

        Ok(ledger)
    }

    /// Deletes segments that have fallen past the retention horizon, returning how many went.
    ///
    /// Whole files only. A segment is kept until every record it could hold is expired, so a
    /// little more than the horizon survives — erring towards keeping data rather than towards
    /// deleting something still inside the window.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the directory cannot be listed or a file cannot be removed.
    pub fn prune(&self, horizon: Timestamp) -> Result<usize, StoreError> {
        prune_segments(&self.root, RUNS, horizon)
    }

    /// The segment file a record filed at `at` belongs in.
    fn segment_for(&self, at: Timestamp) -> PathBuf {
        segment_for(&self.root, RUNS, at)
    }
}
