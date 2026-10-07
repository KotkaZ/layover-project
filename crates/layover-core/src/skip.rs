//! A scheduled tick that came due and did not start.
//!
//! # Why this is written down
//!
//! A schedule whose work always outlasts its interval skips every tick, and until this was kept it
//! said so only on the Tower's console. Read back from history, such a factory looks healthy: the
//! runs that did happen all succeeded, and nothing records the ones that never began. A person
//! asking "why has the hourly sweep run four times today" had nowhere to look.
//!
//! So a skip is kept beside the stalls, as a dated event, and the dashboard reads it.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::pipeline::PipelineName;

/// Why a tick that came due did not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The pipeline's previous wave was still queued or running, and it does not allow overlap.
    StillWorking,
}

impl SkipReason {
    /// The stable name used on disk and in the API.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::StillWorking => "still_working",
        }
    }
}

/// One tick of a schedule that came due and did not start.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Skip {
    /// The pipeline whose tick it was.
    pub pipeline: PipelineName,
    /// When the Tower decided not to start it.
    pub at: Timestamp,
    /// Why.
    pub reason: SkipReason,
}

impl Skip {
    /// A tick skipped because the pipeline's previous wave had not finished.
    #[must_use]
    pub fn still_working(pipeline: PipelineName, at: Timestamp) -> Self {
        Self {
            pipeline,
            at,
            reason: SkipReason::StillWorking,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skip_survives_a_round_trip_through_json() {
        // Written to disk because a tick that never started leaves nothing else behind.
        let original = Skip::still_working(
            PipelineName::new("sweep"),
            "2026-10-07T09:00:00Z".parse().expect("valid"),
        );
        let text = serde_json::to_string(&original).expect("serialises");

        assert!(text.contains(r#""reason":"still_working""#), "{text}");
        let read: Skip = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(read, original);
        assert_eq!(read.reason.slug(), "still_working");
    }
}
