//! The numbers a learning lives by, which a factory may set in `[learnings]`.
//!
//! Each default was chosen for a reason given beside its constant in the parent module; a factory
//! that runs an agent a hundred times a day, or once a week, has a different answer to "how many
//! runs is long enough to tell a fluke from a fact", so the numbers are its to change. What is not
//! here stays fixed: how two learnings are judged the same, which fails silently in both
//! directions when it is wrong, and what a learning may not say, which is a safety screen rather
//! than a preference.

use super::{CONFIRM_AFTER, MAX_TEXT, PROVISIONAL_RUNS};

/// How learnings are taken up, how long they last, and when they become permanent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rules {
    /// Runs of its agent a provisional learning survives before it lapses.
    pub provisional_runs: u32,
    /// Independent rediscoveries — proposals arriving while it is lapsed — that make a learning
    /// permanent.
    pub confirm_after: u32,
    /// The longest a learning may be, in characters.
    pub max_chars: usize,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            provisional_runs: PROVISIONAL_RUNS,
            confirm_after: CONFIRM_AFTER,
            max_chars: MAX_TEXT,
        }
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use super::*;
    use crate::learning::{Impact, Learnings, Proposal, State, Uptake};

    fn proposal(text: &str) -> Proposal {
        Proposal::new(
            "eagle".into(),
            text,
            Impact::Medium,
            "2026-10-01T09:00:00Z".parse().expect("valid"),
        )
    }

    fn charge(learnings: &mut Learnings, runs: u32) {
        let at: Timestamp = "2026-10-02T09:00:00Z".parse().expect("valid");
        for _ in 0..runs {
            learnings.charge_run(&"eagle".into(), at);
        }
    }

    #[test]
    fn a_factory_sets_how_many_runs_a_learning_lasts() {
        let rules = Rules {
            provisional_runs: 5,
            ..Rules::default()
        };
        let mut learnings = Learnings::new();
        learnings.propose_under(&proposal("use ripgrep here"), &rules);
        assert_eq!(learnings.all().next().expect("one").runs_left, 5);

        charge(&mut learnings, 4);
        assert_eq!(
            learnings.all().next().expect("one").state,
            State::Provisional
        );
        charge(&mut learnings, 1);
        assert_eq!(learnings.all().next().expect("one").state, State::Lapsed);

        learnings.propose_under(&proposal("use ripgrep here"), &rules);
        assert_eq!(
            learnings.all().next().expect("one").runs_left,
            5,
            "a rediscovery's second life is as long as the factory says"
        );
    }

    #[test]
    fn a_factory_sets_how_many_rediscoveries_make_a_learning_permanent() {
        let rules = Rules {
            provisional_runs: 1,
            confirm_after: 1,
            ..Rules::default()
        };
        let mut learnings = Learnings::new();
        learnings.propose_under(&proposal("use ripgrep here"), &rules);
        charge(&mut learnings, 1);

        assert_eq!(
            learnings.propose_under(&proposal("use ripgrep here"), &rules),
            Uptake::Confirmed
        );
    }

    #[test]
    fn a_factory_sets_how_long_a_learning_may_be() {
        let rules = Rules {
            max_chars: 20,
            ..Rules::default()
        };
        let mut learnings = Learnings::new();

        assert_eq!(
            learnings.propose_under(&proposal("this sentence is longer than twenty"), &rules),
            Uptake::Malformed
        );
        assert_eq!(
            learnings.propose_under(&proposal("use ripgrep here"), &rules),
            Uptake::Taken
        );
    }

    #[test]
    fn the_defaults_are_the_documented_numbers() {
        assert_eq!(
            Rules::default(),
            Rules {
                provisional_runs: 20,
                confirm_after: 3,
                max_chars: 400,
            }
        );
    }
}
