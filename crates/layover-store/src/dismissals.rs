//! Stalled and failed chains a person has dismissed.
//!
//! State rather than events, like the switches beside it: one line per dismissed chain, in a
//! single document rewritten whole, and no file until somebody dismisses something. Undoing a
//! dismissal removes its line.
//!
//! A dismissal names the run the chain had ended on, because that is what was looked at and
//! judged. The chain itself may go on — a restart, a continuation — and end badly again, and that
//! is a new failure nobody has seen. Reading which runs a chain has is the dashboard's business;
//! this only keeps what was said.

use std::path::PathBuf;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use layover_core::flight::{ItineraryId, RunId};
use serde::{Deserialize, Serialize};

use crate::Journal;
use crate::history::StoreError;
use crate::journal::poisoned;

/// A person's word that a stalled or failed chain has been dealt with.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Dismissal {
    /// The chain.
    pub itinerary: ItineraryId,
    /// The run it had ended on when it was dismissed.
    pub run: RunId,
    /// Who dismissed it.
    pub by: String,
    /// When.
    pub at: Timestamp,
    /// Why, when they said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Journal {
    /// Every dismissal, once per chain, in the order they were made.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the file exists but cannot be read.
    pub fn dismissals(&self) -> Result<Vec<Dismissal>, StoreError> {
        let mut read: Vec<Dismissal> = crate::segment::read_document(&self.dismissals_path())?;
        // A line added by hand for a chain already dismissed is the same dismissal, not a second.
        let mut seen = std::collections::BTreeSet::new();
        read.retain(|dismissal| seen.insert(dismissal.itinerary.clone()));
        Ok(read)
    }

    /// Records `dismissal`, and says which dismissal of its chain now stands.
    ///
    /// Dismissing the same run twice keeps the first, so a second press moves neither who nor
    /// when. Dismissing a later run of the chain replaces the earlier dismissal, which no longer
    /// applies to anything.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the file cannot be read or written.
    pub fn dismiss(&self, dismissal: Dismissal) -> Result<Dismissal, StoreError> {
        let _writing = self.writes.lock().map_err(|_| poisoned())?;

        let mut kept = self.dismissals()?;
        if let Some(already) = kept
            .iter()
            .find(|kept| kept.itinerary == dismissal.itinerary && kept.run == dismissal.run)
        {
            return Ok(already.clone());
        }

        kept.retain(|kept| kept.itinerary != dismissal.itinerary);
        kept.push(dismissal.clone());
        crate::segment::write_document(&self.dismissals_path(), &kept)?;
        Ok(dismissal)
    }

    /// Undoes `itinerary`'s dismissal, reporting whether it had one.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the file cannot be read or written.
    pub fn undismiss(&self, itinerary: &ItineraryId) -> Result<bool, StoreError> {
        let _writing = self.writes.lock().map_err(|_| poisoned())?;

        let kept = self.dismissals()?;
        let before = kept.len();
        let kept: Vec<Dismissal> = kept
            .into_iter()
            .filter(|kept| kept.itinerary != *itinerary)
            .collect();

        let had = kept.len() != before;
        if had {
            crate::segment::write_document(&self.dismissals_path(), &kept)?;
        }
        Ok(had)
    }

    /// Forgets dismissals made before the first day retention keeps, returning how many went.
    ///
    /// The same rule as the run segments, and safe for the same reason: a run is filed by the day
    /// it finished, a chain is dismissed after its run finished, so a dismissal goes no sooner than
    /// the run it was about.
    pub(crate) fn forget_dismissals(&self, horizon: Timestamp) -> Result<usize, StoreError> {
        let _writing = self.writes.lock().map_err(|_| poisoned())?;

        let cutoff = crate::segment::kept_from(horizon);
        let kept = self.dismissals()?;
        let before = kept.len();
        let kept: Vec<Dismissal> = kept
            .into_iter()
            .filter(|kept| kept.at.to_zoned(TimeZone::UTC).date() >= cutoff)
            .collect();

        let forgotten = before - kept.len();
        if forgotten > 0 {
            crate::segment::write_document(&self.dismissals_path(), &kept)?;
        }
        Ok(forgotten)
    }

    /// Where dismissals are kept.
    fn dismissals_path(&self) -> PathBuf {
        self.root().join("dismissed.jsonl")
    }
}
