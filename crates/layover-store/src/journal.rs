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
use layover_core::help::{Blocker, HelpRequest};
use layover_core::learning::{Learning, Learnings};

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
        crate::segment::prune_segments(&self.root, "help", horizon)
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
        let path = self.learnings_path();
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Learnings::new());
            }
            Err(error) => return Err(StoreError::at(&path)(error)),
        };

        let entries: Vec<Learning> = raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        Ok(Learnings::from_entries(entries))
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
        let path = self.learnings_path();
        let staging = path.with_extension("jsonl.writing");

        let mut body = String::new();
        for learning in learnings.all() {
            body.push_str(&serde_json::to_string(learning)?);
            body.push('\n');
        }

        fs::write(&staging, body).map_err(StoreError::at(&staging))?;
        fs::rename(&staging, &path).map_err(StoreError::at(&path))
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
