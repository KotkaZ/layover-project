//! `[learnings]`: the numbers a learning lives by — how long it applies, what makes it permanent,
//! how long it may be, how many a run is given, and how long it is kept once it lapses.

use jiff::{SignedDuration, Timestamp};
use serde::Deserialize;

use crate::learning::{CONFIRM_AFTER, MAX_TEXT, PROVISIONAL_RUNS, Rules};

/// How learnings are taken up, given to runs, and let go.
///
/// Every key is optional and defaults to the number Layover has always used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningsConfig {
    /// Runs of its agent a new or rediscovered learning applies for before it lapses.
    #[serde(default = "default_provisional_runs")]
    pub provisional_runs: u32,
    /// Rediscoveries — proposals of a learning while it is lapsed — that make it permanent.
    #[serde(default = "default_confirm_after")]
    pub confirm_after: u32,
    /// The longest a learning may be, in characters. A longer proposal is refused.
    #[serde(default = "default_max_chars")]
    pub max_chars: u32,
    /// The most learnings one run is given; confirmed ones first, then the oldest.
    #[serde(default = "default_max_per_run")]
    pub max_per_run: u32,
    /// Days a lapsed learning is kept before it is deleted. `0` deletes it as soon as it lapses.
    ///
    /// Kept, a lapsed learning is what lets a later proposal of the same insight count as a
    /// rediscovery; deleted, that proposal starts a new learning. Longer gives a learning more
    /// chance to be rediscovered three times and confirmed; shorter keeps the list to what is in
    /// use.
    /// Written `keep_lapsed = "14d"` or `keep_lapsed_days = 14`.
    #[serde(
        default = "default_keep_lapsed_days",
        alias = "keep_lapsed",
        deserialize_with = "crate::duration::days"
    )]
    pub keep_lapsed_days: u32,
}

/// Two weeks: long enough for an agent that runs daily to arrive at a true insight again, short
/// enough that a factory lapsing nine learnings a day keeps about a hundred and twenty.
fn default_keep_lapsed_days() -> u32 {
    14
}

fn default_provisional_runs() -> u32 {
    PROVISIONAL_RUNS
}

fn default_confirm_after() -> u32 {
    CONFIRM_AFTER
}

fn default_max_chars() -> u32 {
    u32::try_from(MAX_TEXT).unwrap_or(u32::MAX)
}

fn default_max_per_run() -> u32 {
    u32::try_from(crate::brief::MAX_INJECTED).unwrap_or(u32::MAX)
}

impl Default for LearningsConfig {
    fn default() -> Self {
        Self {
            provisional_runs: default_provisional_runs(),
            confirm_after: default_confirm_after(),
            max_chars: default_max_chars(),
            max_per_run: default_max_per_run(),
            keep_lapsed_days: default_keep_lapsed_days(),
        }
    }
}

impl LearningsConfig {
    /// The rules a proposal is taken up under.
    #[must_use]
    pub fn rules(&self) -> Rules {
        Rules {
            provisional_runs: self.provisional_runs,
            confirm_after: self.confirm_after,
            max_chars: usize::try_from(self.max_chars).unwrap_or(usize::MAX),
        }
    }

    /// The most learnings one run is given.
    #[must_use]
    pub fn per_run(&self) -> usize {
        usize::try_from(self.max_per_run).unwrap_or(usize::MAX)
    }

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
    fn a_factory_that_says_nothing_gets_the_numbers_layover_has_always_used() {
        let config = Config::from_toml("", "learnings.toml").expect("parses");

        assert_eq!(config.learnings.rules(), Rules::default());
        assert_eq!(config.learnings.per_run(), 25);
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
    fn every_number_can_be_set() {
        let config = Config::from_toml(
            "[learnings]\nprovisional_runs = 50\nconfirm_after = 2\nmax_chars = 280\n\
             max_per_run = 10\nkeep_lapsed_days = 30\n",
            "learnings.toml",
        )
        .expect("parses");

        assert_eq!(
            config.learnings.rules(),
            Rules {
                provisional_runs: 50,
                confirm_after: 2,
                max_chars: 280,
            }
        );
        assert_eq!(config.learnings.per_run(), 10);
        assert_eq!(config.learnings.keep_lapsed_days, 30);
    }

    #[test]
    fn an_unknown_key_is_refused() {
        assert!(
            Config::from_toml("[learnings]\nkeep_lapsed_weeks = 3\n", "learnings.toml").is_err()
        );
    }
}
