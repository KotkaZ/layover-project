//! `[learnings]`: how long a learning that lapsed is kept before it is deleted.

use jiff::{SignedDuration, Timestamp};
use serde::Deserialize;

/// What becomes of learnings once they stop applying.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningsConfig {
    /// Days a lapsed learning is kept before it is deleted. `0` deletes it as soon as it lapses.
    ///
    /// Kept, a lapsed learning is what lets a later proposal of the same insight count as a
    /// rediscovery; deleted, that proposal starts a new learning. Longer gives a learning more
    /// chance to be rediscovered three times and confirmed; shorter keeps the list to what is in
    /// use.
    #[serde(default = "default_keep_lapsed_days")]
    pub keep_lapsed_days: u32,
}

/// Two weeks: long enough for an agent that runs daily to arrive at a true insight again, short
/// enough that a factory lapsing nine learnings a day keeps about a hundred and twenty.
fn default_keep_lapsed_days() -> u32 {
    14
}

impl Default for LearningsConfig {
    fn default() -> Self {
        Self {
            keep_lapsed_days: default_keep_lapsed_days(),
        }
    }
}

impl LearningsConfig {
    /// The latest a learning may have lapsed and be deleted `now`.
    #[must_use]
    pub fn forget_lapsed_before(&self, now: Timestamp) -> Timestamp {
        now.checked_sub(SignedDuration::from_hours(
            i64::from(self.keep_lapsed_days) * 24,
        ))
        .unwrap_or(Timestamp::MIN)
    }

    /// When a learning that lapsed `at` is deleted.
    #[must_use]
    pub fn deleted_after(&self, at: Timestamp) -> Timestamp {
        at.checked_add(SignedDuration::from_hours(
            i64::from(self.keep_lapsed_days) * 24,
        ))
        .unwrap_or(Timestamp::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn at(rfc3339: &str) -> Timestamp {
        rfc3339.parse().expect("valid timestamp")
    }

    #[test]
    fn a_factory_that_says_nothing_keeps_lapsed_learnings_for_two_weeks() {
        let config = Config::from_toml("", "learnings.toml").expect("parses");

        assert_eq!(config.learnings.keep_lapsed_days, 14);
        assert_eq!(
            config
                .learnings
                .forget_lapsed_before(at("2026-10-15T09:00:00Z")),
            at("2026-10-01T09:00:00Z")
        );
    }

    #[test]
    fn zero_deletes_a_learning_the_moment_it_lapses() {
        let config = Config::from_toml("[learnings]\nkeep_lapsed_days = 0\n", "learnings.toml")
            .expect("parses");
        let now = at("2026-10-15T09:00:00Z");

        assert_eq!(config.learnings.forget_lapsed_before(now), now);
        assert_eq!(config.learnings.deleted_after(now), now);
    }

    #[test]
    fn an_unknown_key_is_refused() {
        assert!(Config::from_toml("[learnings]\nkeep_lapsed = 3\n", "learnings.toml").is_err());
    }
}
