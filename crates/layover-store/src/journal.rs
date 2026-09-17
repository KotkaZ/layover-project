//! Help requests and learnings on disk.
//!
//! Same discipline as run history — append-only JSON Lines, readable in an editor — but two
//! different shapes, because the two things are asked about differently.
//!
//! Help requests are **events**: each one happened at a moment and is answered or not. They are
//! segmented by day like runs, and pruned on the same horizon.
//!
//! Learnings are **state**: one record per insight, rewritten as it is rediscovered, lapses or is
//! revoked. Segmenting those by day would scatter one learning's life across ninety files. They
//! live in a single document, rewritten whole, which is affordable because there are hundreds of
//! them rather than millions — and is why they are *not* subject to the retention horizon. A
//! confirmed learning that expired because it was ninety days old would be the one thing in the
//! system that got worse the longer it was right.

use std::fs;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use layover_core::agent::AgentName;
use layover_core::cost::Span;
use layover_core::flight::{Flight, FlightId};
use layover_core::help::{Blocker, HelpRequest};
use layover_core::layover::{Layover, LayoverId, Standing};
use layover_core::learning::{Learning, Learnings};
use layover_core::report::Report;

use crate::history::StoreError;

/// Which help requests to return.
#[derive(Debug, Clone, Default)]
pub struct HelpFilter {
    /// Only requests from this agent.
    pub agent: Option<AgentName>,
    /// Only requests of this kind.
    pub blocker: Option<Blocker>,
    /// Only requests nobody has dealt with.
    pub open_only: bool,
    /// Only requests that stopped the work.
    pub fatal_only: bool,
}

impl HelpFilter {
    /// Returns `true` when `request` passes every narrowing.
    fn matches(&self, request: &HelpRequest) -> bool {
        self.agent.as_ref().is_none_or(|a| *a == request.agent)
            && self.blocker.is_none_or(|b| b == request.blocker)
            && (!self.open_only || request.is_open())
            && (!self.fatal_only || request.fatal)
    }
}

/// The help requests and learnings a factory has accumulated.
#[derive(Debug, Clone)]
pub struct Journal {
    root: PathBuf,
}

impl Journal {
    /// Opens — and creates, if needed — a journal directory.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the directory cannot be created.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(StoreError::at(&root))?;
        Ok(Self { root })
    }

    /// Where the journal lives.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Files a help request.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if it cannot be written, or [`StoreError::Serialise`] if it
    /// cannot be encoded.
    pub fn ask(&self, request: &HelpRequest) -> Result<(), StoreError> {
        let path = self.help_segment(request.at);
        crate::segment::append_line(&path, &serde_json::to_string(request)?)
    }

    /// Reads the help requests raised inside `span`, most recent first.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if a segment exists but cannot be read.
    pub fn help(&self, span: &Span, filter: &HelpFilter) -> Result<Vec<HelpRequest>, StoreError> {
        let mut found: Vec<HelpRequest> = crate::segment::read_segments(&self.root, "help", span)?
            .into_iter()
            .filter(|request: &HelpRequest| span.contains(request.at) && filter.matches(request))
            .collect();

        found.sort_by_key(|request| std::cmp::Reverse(request.at));
        Ok(found)
    }

    /// Marks every open request matching `filter` as dealt with, returning how many.
    ///
    /// Rewrites the segments it touches. Help requests are the one append-only thing here that
    /// genuinely changes after the fact: a blocker gets fixed, and a list that cannot be cleared
    /// is a list nobody reads twice.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if a segment cannot be read or rewritten.
    pub fn resolve(
        &self,
        span: &Span,
        filter: &HelpFilter,
        at: Timestamp,
    ) -> Result<usize, StoreError> {
        let mut resolved = 0;

        for path in crate::segment::segments_covering(&self.root, "help", span)? {
            let mut requests: Vec<HelpRequest> = crate::segment::read_segment(&path)?;
            let mut touched = false;

            for request in &mut requests {
                if request.is_open() && span.contains(request.at) && filter.matches(request) {
                    request.resolved_at = Some(at);
                    resolved += 1;
                    touched = true;
                }
            }

            if touched {
                crate::segment::rewrite_segment(&path, &requests)?;
            }
        }

        Ok(resolved)
    }

    /// Deletes help segments past the retention horizon, returning how many went.
    ///
    /// Learnings are deliberately untouched: one that expired for being old would be the single
    /// thing here that got worse the longer it was right.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the directory cannot be listed or a file cannot be removed.
    pub fn prune(&self, horizon: Timestamp) -> Result<usize, StoreError> {
        let help = crate::segment::prune_segments(&self.root, "help", horizon)?;
        let reports = crate::segment::prune_segments(&self.root, "reports", horizon)?;
        Ok(help + reports)
    }

    /// Reads the learnings.
    ///
    /// A missing file is an empty collection, not an error: every factory starts having learned
    /// nothing.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the file exists but cannot be read.
    pub fn learnings(&self) -> Result<Learnings, StoreError> {
        Ok(Learnings::from_entries(crate::segment::read_document(
            &self.learnings_path(),
        )?))
    }

    /// Writes the learnings back.
    ///
    /// Written to a neighbouring file and renamed, so an interrupted write cannot leave the
    /// factory with half its memory. The run history can tolerate a torn final line because a
    /// line is one record; here the file *is* the record.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if it cannot be written or moved into place.
    pub fn save_learnings(&self, learnings: &Learnings) -> Result<(), StoreError> {
        let entries: Vec<&Learning> = learnings.all().collect();
        crate::segment::write_document(&self.learnings_path(), &entries)
    }

    /// Where the learnings live.
    fn learnings_path(&self) -> PathBuf {
        self.root.join("learnings.jsonl")
    }

    /// The help segment a request raised at `at` belongs in.
    fn help_segment(&self, at: Timestamp) -> PathBuf {
        let date = at.to_zoned(TimeZone::UTC).date();
        self.root.join(format!("help-{date}.jsonl"))
    }
}

impl Journal {
    /// Reads the booked layovers.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the file exists but cannot be read.
    pub fn layovers(&self) -> Result<Vec<Layover>, StoreError> {
        crate::segment::read_document(&self.layovers_path())
    }

    /// Writes the layovers back.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if it cannot be written, or [`StoreError::Serialise`] if a
    /// layover cannot be encoded.
    pub fn save_layovers(&self, layovers: &[Layover]) -> Result<(), StoreError> {
        crate::segment::write_document(&self.layovers_path(), layovers)
    }

    /// Books a layover.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the file cannot be read back or written.
    pub fn book(&self, layover: Layover) -> Result<(), StoreError> {
        let mut booked = self.layovers()?;
        booked.push(layover);
        self.save_layovers(&booked)
    }

    /// The layovers due to be picked up at `now`, soonest first.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the file cannot be read.
    pub fn due(&self, now: Timestamp) -> Result<Vec<Layover>, StoreError> {
        let mut due: Vec<Layover> = self
            .layovers()?
            .into_iter()
            .filter(|layover| layover.is_due(now))
            .collect();

        due.sort_by_key(|layover| layover.due_at);
        Ok(due)
    }

    /// Applies a change to one layover, reporting whether it was there.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the file cannot be read or written.
    pub fn amend(
        &self,
        id: &LayoverId,
        change: impl FnOnce(&mut Layover),
    ) -> Result<bool, StoreError> {
        let mut booked = self.layovers()?;
        let Some(layover) = booked.iter_mut().find(|layover| layover.id == *id) else {
            return Ok(false);
        };

        change(layover);
        self.save_layovers(&booked)?;
        Ok(true)
    }

    /// Drops layovers that are finished with and older than `horizon`.
    ///
    /// Only the ones nothing will pick up again. A booked layover is kept however old it is,
    /// because its age is exactly what makes it interesting — something still waiting after two
    /// months is a question, not rubbish.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the file cannot be read or written.
    pub fn sweep(&self, horizon: Timestamp) -> Result<usize, StoreError> {
        let booked = self.layovers()?;
        let was = booked.len();

        let kept: Vec<Layover> = booked
            .into_iter()
            .filter(|layover| layover.standing == Standing::Booked || layover.booked_at >= horizon)
            .collect();

        let removed = was - kept.len();
        if removed > 0 {
            self.save_layovers(&kept)?;
        }
        Ok(removed)
    }

    /// Where the layovers live.
    fn layovers_path(&self) -> PathBuf {
        self.root.join("layovers.jsonl")
    }
}

impl Journal {
    /// Files an agent's report on its own run.
    ///
    /// Segmented by day like help requests, because a report is an event: it describes one run at
    /// one moment and never changes afterwards.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if it cannot be written, or [`StoreError::Serialise`] if it
    /// cannot be encoded.
    pub fn file(&self, report: &Report) -> Result<(), StoreError> {
        let path = crate::segment::segment_for(&self.root, "reports", report.at);
        crate::segment::append_line(&path, &serde_json::to_string(report)?)
    }

    /// Reads the reports written inside `span`, most recent first.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if a segment exists but cannot be read.
    pub fn reports(&self, span: &Span) -> Result<Vec<Report>, StoreError> {
        let mut found: Vec<Report> = crate::segment::read_segments(&self.root, "reports", span)?
            .into_iter()
            .filter(|report: &Report| span.contains(report.at))
            .collect();

        found.sort_by_key(|report| std::cmp::Reverse(report.at));
        Ok(found)
    }

    /// The report for one run, if it wrote one.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if a segment cannot be read.
    pub fn report_for(&self, span: &Span, run: &str) -> Result<Option<Report>, StoreError> {
        Ok(self
            .reports(span)?
            .into_iter()
            .find(|report| report.run.as_str() == run))
    }

    /// Queues a flight a human asked for.
    ///
    /// Deliberately not a new concept. A manual trigger is a flight that has not been dispatched
    /// yet, so it is stored as one; inventing a separate "request" would add a noun to a
    /// vocabulary that already has more than anybody can hold.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the queue cannot be read back or written.
    pub fn queue(&self, flight: Flight) -> Result<(), StoreError> {
        let mut pending = self.pending()?;
        pending.push(flight);
        crate::segment::write_document(&self.pending_path(), &pending)
    }

    /// Flights waiting for something to dispatch them, oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the queue cannot be read.
    pub fn pending(&self) -> Result<Vec<Flight>, StoreError> {
        crate::segment::read_document(&self.pending_path())
    }

    /// Removes a queued flight, reporting whether it was there.
    ///
    /// The Tower will call this as it picks work up. Until one exists it is how a human cancels
    /// something they queued by mistake.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the queue cannot be read or written.
    pub fn unqueue(&self, id: &FlightId) -> Result<bool, StoreError> {
        let pending = self.pending()?;
        let kept: Vec<Flight> = pending
            .into_iter()
            .filter(|flight| flight.id != *id)
            .collect();

        let removed = kept.len() != self.pending()?.len();
        if removed {
            crate::segment::write_document(&self.pending_path(), &kept)?;
        }
        Ok(removed)
    }

    /// Where queued flights live.
    fn pending_path(&self) -> PathBuf {
        self.root.join("pending.jsonl")
    }
}
