//! What reconciliation concludes about a run recorded as live.
//!
//! The record itself — [`Live`], kept by a [`Ledger`] — lives in `layover-store`, because the
//! dashboard reads it too: it is how the dashboard knows what is running, whichever process
//! started it. They are re-exported here, where they were first defined.

pub use layover_store::live::{Ledger, Live};

/// What reconciliation concluded about one recorded run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The process is still running: same identifier, same start time.
    StillRunning,
    /// Nothing is running under that identifier. The run was interrupted and is recoverable.
    ConfirmedGone,
    /// Something is running under that identifier, but it did not start when this run did.
    ///
    /// The identifier was reused. The original run is gone, and the supervisor must not wait for
    /// whatever inherited its number.
    Reused,
}

impl Verdict {
    /// Whether recovery may start a replacement for this run.
    ///
    /// Recovery requires the previous process to be *confirmed* gone. A reused identifier is also
    /// gone — that is why it is a separate verdict rather than an error: the conclusion is the
    /// same, the evidence is different, and saying so makes the log readable.
    #[must_use]
    pub const fn is_recoverable(self) -> bool {
        matches!(self, Self::ConfirmedGone | Self::Reused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reused_identifier_is_still_recoverable_but_says_why() {
        assert!(Verdict::ConfirmedGone.is_recoverable());
        assert!(Verdict::Reused.is_recoverable());
        assert!(!Verdict::StillRunning.is_recoverable());
    }
}
