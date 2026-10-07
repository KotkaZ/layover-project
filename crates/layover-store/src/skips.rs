//! Scheduled ticks that came due and did not start, kept as dated events beside the stalls.

use layover_core::cost::Span;
use layover_core::skip::Skip;

use crate::Journal;
use crate::history::StoreError;

impl Journal {
    /// Records a scheduled tick that came due and did not start.
    ///
    /// A day segment, like a stall: a dated event somebody needs to see, pruned on the same
    /// horizon as the runs that did happen.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the segment cannot be written.
    pub fn record_skip(&self, skip: &Skip) -> Result<(), StoreError> {
        let path = crate::segment::segment_for(self.root(), "skips", skip.at);
        let line = serde_json::to_string(skip)?;
        crate::segment::append_line(&path, &line)
    }

    /// Ticks skipped within `span`, most recent first.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if a segment cannot be read.
    pub fn skips(&self, span: &Span) -> Result<Vec<Skip>, StoreError> {
        let mut found: Vec<Skip> = crate::segment::read_segments(self.root(), "skips", span)?
            .into_iter()
            .filter(|skip: &Skip| span.contains(skip.at))
            .collect();

        found.sort_by_key(|skip| std::cmp::Reverse(skip.at));
        Ok(found)
    }
}
