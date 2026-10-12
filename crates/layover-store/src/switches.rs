//! Workflows a person has switched off.
//!
//! State rather than events, and small: one line per workflow that is off, in a single document
//! rewritten whole. A workflow that is on has no line, so a factory nobody has switched anything
//! in has no file at all, and deleting the file switches everything back on.
//!
//! Kept here rather than in `layover.toml` because a switch is something a person flips while the
//! factory runs — the Tower reads it on every pass — and because one flipped in the dashboard must
//! survive the restart that reloads the configuration. A line can be added by hand, with or
//! without `since`, when nothing is answering.

use std::path::PathBuf;

use jiff::Timestamp;
use layover_core::pipeline::PipelineName;
use serde::{Deserialize, Serialize};

use crate::Journal;
use crate::history::StoreError;
use crate::journal::poisoned;

/// One workflow that is switched off, and since when.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SwitchedOff {
    /// The workflow.
    pub pipeline: PipelineName,
    /// When it was switched off. Absent from a line written by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<Timestamp>,
}

impl Journal {
    /// Every workflow that is switched off, once each, in the order they were.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the file exists but cannot be read.
    pub fn switched_off(&self) -> Result<Vec<SwitchedOff>, StoreError> {
        let mut read: Vec<SwitchedOff> = crate::segment::read_document(&self.switches_path())?;
        // A line added by hand for a workflow already off is the same switch, not a second one.
        let mut seen = std::collections::BTreeSet::new();
        read.retain(|off| seen.insert(off.pipeline.clone()));
        Ok(read)
    }

    /// Switches `pipeline` off, and says since when it is.
    ///
    /// Switching off a workflow that is already off changes nothing and is not an error: somebody
    /// pressing the switch twice should not be told it failed, nor have "off since" move.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the file cannot be read or written.
    pub fn switch_off(
        &self,
        pipeline: &PipelineName,
        at: Timestamp,
    ) -> Result<SwitchedOff, StoreError> {
        let _writing = self.writes.lock().map_err(|_| poisoned())?;

        let mut off = self.switched_off()?;
        if let Some(already) = off.iter().find(|off| off.pipeline == *pipeline) {
            return Ok(already.clone());
        }

        let now = SwitchedOff {
            pipeline: pipeline.clone(),
            since: Some(at),
        };
        off.push(now.clone());
        crate::segment::write_document(&self.switches_path(), &off)?;
        Ok(now)
    }

    /// Switches `pipeline` back on, reporting whether it was off.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] if the file cannot be read or written.
    pub fn switch_on(&self, pipeline: &PipelineName) -> Result<bool, StoreError> {
        let _writing = self.writes.lock().map_err(|_| poisoned())?;

        let off = self.switched_off()?;
        let before = off.len();
        let kept: Vec<SwitchedOff> = off
            .into_iter()
            .filter(|off| off.pipeline != *pipeline)
            .collect();

        let was_off = kept.len() != before;
        if was_off {
            crate::segment::write_document(&self.switches_path(), &kept)?;
        }
        Ok(was_off)
    }

    /// Where switched-off workflows are kept.
    fn switches_path(&self) -> PathBuf {
        self.root().join("switched-off.jsonl")
    }
}
