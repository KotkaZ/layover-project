//! Deleting learnings that lapsed and were not rediscovered in time.
//!
//! A lapsed learning is kept for one reason: a proposal that matches it counts as a rediscovery,
//! and three of those make it permanent. One nobody rediscovers within `keep_lapsed_days` was most
//! likely a fluke, and kept for ever it is only a longer list somebody has to read past — a real
//! factory lapsed over a hundred in twelve days and rediscovered one. So it is deleted.
//!
//! Only lapsed learnings go. A rejected one stays, because it is what stops an agent proposing
//! again something a person refused; provisional and confirmed ones are still in use.

use jiff::Timestamp;

use super::{LearningId, Learnings, State};

impl Learnings {
    /// Deletes every lapsed learning that lapsed at or before `before`, and says which.
    ///
    /// A learning that lapsed before Layover recorded when counts from when it was last proposed.
    pub fn forget_lapsed(&mut self, before: Timestamp) -> Vec<LearningId> {
        let mut forgotten = Vec::new();
        self.entries.retain(|learning| {
            let stale = learning.state == State::Lapsed
                && learning.lapsed_at.unwrap_or(learning.last_at) <= before;
            if stale {
                forgotten.push(learning.id.clone());
            }
            !stale
        });
        forgotten
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::{Impact, PROVISIONAL_RUNS, Proposal, Uptake};

    fn at(rfc3339: &str) -> Timestamp {
        rfc3339.parse().expect("valid timestamp")
    }

    fn propose(learnings: &mut Learnings, text: &str, when: &str) -> Uptake {
        learnings.propose(&Proposal::new(
            "eagle".into(),
            text,
            Impact::Medium,
            at(when),
        ))
    }

    fn lapse(learnings: &mut Learnings, when: &str) {
        for _ in 0..PROVISIONAL_RUNS {
            learnings.charge_run(&"eagle".into(), at(when));
        }
    }

    fn texts(learnings: &Learnings) -> Vec<&str> {
        learnings
            .all()
            .map(|learning| learning.text.as_str())
            .collect()
    }

    #[test]
    fn a_learning_that_lapsed_records_when() {
        let mut learnings = Learnings::new();
        propose(&mut learnings, "use ripgrep here", "2026-10-01T09:00:00Z");
        lapse(&mut learnings, "2026-10-03T12:00:00Z");

        let learning = learnings.all().next().expect("one");
        assert_eq!(learning.state, State::Lapsed);
        assert_eq!(learning.lapsed_at, Some(at("2026-10-03T12:00:00Z")));
    }

    #[test]
    fn a_lapsed_learning_is_deleted_once_its_days_are_up_and_not_before() {
        let mut learnings = Learnings::new();
        propose(&mut learnings, "use ripgrep here", "2026-10-01T09:00:00Z");
        lapse(&mut learnings, "2026-10-03T12:00:00Z");

        assert!(
            learnings
                .forget_lapsed(at("2026-10-03T11:59:59Z"))
                .is_empty(),
            "lapsed after the cut-off, so it is kept"
        );
        assert_eq!(learnings.len(), 1);

        let forgotten = learnings.forget_lapsed(at("2026-10-03T12:00:00Z"));
        assert_eq!(forgotten.len(), 1);
        assert!(learnings.is_empty());
    }

    #[test]
    fn only_lapsed_learnings_are_deleted() {
        let mut learnings = Learnings::new();
        propose(&mut learnings, "use ripgrep here", "2026-10-01T09:00:00Z");
        lapse(&mut learnings, "2026-10-02T09:00:00Z");
        propose(
            &mut learnings,
            "pin the toolchain first",
            "2026-10-03T09:00:00Z",
        );
        propose(&mut learnings, "never force push", "2026-10-03T09:00:00Z");
        propose(
            &mut learnings,
            "always rerun flaky suites",
            "2026-10-03T09:00:00Z",
        );
        let ids: Vec<LearningId> = learnings.all().map(|l| l.id.clone()).collect();
        learnings.confirm(&ids[2], at("2026-10-03T10:00:00Z"));
        learnings.reject(&ids[3], at("2026-10-03T10:00:00Z"));

        learnings.forget_lapsed(at("2026-12-31T00:00:00Z"));

        assert_eq!(
            texts(&learnings),
            [
                "pin the toolchain first",
                "never force push",
                "always rerun flaky suites"
            ],
            "provisional and confirmed are in use, and a rejection stops it coming back"
        );
    }

    #[test]
    fn a_rediscovery_within_the_days_revives_it_and_it_is_no_longer_lapsed() {
        let mut learnings = Learnings::new();
        propose(&mut learnings, "use ripgrep here", "2026-10-01T09:00:00Z");
        lapse(&mut learnings, "2026-10-02T09:00:00Z");

        assert_eq!(
            propose(&mut learnings, "use ripgrep here", "2026-10-05T09:00:00Z"),
            Uptake::Rediscovered { proposals: 2 }
        );
        assert_eq!(learnings.all().next().expect("one").lapsed_at, None);
        assert!(
            learnings
                .forget_lapsed(at("2026-12-31T00:00:00Z"))
                .is_empty()
        );
    }

    #[test]
    fn a_learning_that_lapsed_before_the_time_was_kept_counts_from_its_last_proposal() {
        let mut learnings = Learnings::new();
        propose(&mut learnings, "use ripgrep here", "2026-10-01T09:00:00Z");
        lapse(&mut learnings, "2026-10-04T09:00:00Z");
        learnings.entries[0].lapsed_at = None;

        assert_eq!(learnings.forget_lapsed(at("2026-10-01T09:00:00Z")).len(), 1);
    }

    #[test]
    fn a_lapsed_learning_a_person_keeps_is_no_longer_lapsed() {
        let mut learnings = Learnings::new();
        propose(&mut learnings, "use ripgrep here", "2026-10-01T09:00:00Z");
        lapse(&mut learnings, "2026-10-02T09:00:00Z");
        let id = learnings.all().next().expect("one").id.clone();

        learnings.confirm(&id, at("2026-10-03T09:00:00Z"));

        assert_eq!(learnings.all().next().expect("one").lapsed_at, None);
        assert!(
            learnings
                .forget_lapsed(at("2026-12-31T00:00:00Z"))
                .is_empty()
        );
    }

    #[test]
    fn once_deleted_the_same_insight_is_a_new_learning() {
        let mut learnings = Learnings::new();
        propose(&mut learnings, "use ripgrep here", "2026-10-01T09:00:00Z");
        lapse(&mut learnings, "2026-10-02T09:00:00Z");
        learnings.forget_lapsed(at("2026-10-20T09:00:00Z"));

        assert_eq!(
            propose(&mut learnings, "use ripgrep here", "2026-10-21T09:00:00Z"),
            Uptake::Taken
        );
        assert_eq!(learnings.all().next().expect("one").proposals, 1);
    }
}
