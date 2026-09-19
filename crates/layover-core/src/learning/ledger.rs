//! The collection of learnings, and the rules that move them between states.
//!
//! Three things happen here, and the order they happen in is the design:
//!
//! - a proposal arrives, and is either a new insight, an echo of a live one, or a rediscovery of
//!   a lapsed one;
//! - a run starts, which costs every provisional learning one of its remaining runs;
//! - a human intervenes, which is the rare case rather than the load-bearing one.

use jiff::Timestamp;

use crate::agent::AgentName;

#[cfg(test)]
use super::Impact;

use super::{CONFIRM_AFTER, Learning, LearningId, PROVISIONAL_RUNS, Proposal, State};
use crate::learning::Rejected;

/// What happened to a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Uptake {
    /// Nothing like it was known. It applies from now.
    Taken,
    /// It says the same thing as a learning that is currently being shown to the agent.
    ///
    /// Ignored, because an agent repeating advice it was just given is an echo, not evidence.
    /// Counting it would let a single fluke confirm itself in three runs.
    Echo,
    /// It says the same thing as a learning that had lapsed. Genuine rediscovery, so it counts.
    Rediscovered {
        /// How many independent times it has now been proposed.
        proposals: u32,
    },
    /// Rediscovered often enough to stop expiring.
    Confirmed,
    /// It says the same thing as something a human rejected. Ignored, permanently.
    Refused,
    /// The text was empty or too long.
    Malformed,
    /// The text is not something a learning may say.
    ///
    /// A learning outlives the run that wrote it and is read by every run after, so its text is
    /// screened before it is ever stored rather than filtered on the way out. Storing it and
    /// hiding it later would leave the thing an attacker wanted sitting in the factory's memory,
    /// waiting for the filter to be relaxed.
    Unacceptable(Rejected),
}

/// Every learning the factory holds, across all agents.
#[derive(Debug, Clone, Default)]
pub struct Learnings {
    entries: Vec<Learning>,
}

impl Learnings {
    /// Creates an empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuilds from stored learnings.
    #[must_use]
    pub fn from_entries(entries: Vec<Learning>) -> Self {
        Self { entries }
    }

    /// Every learning, in the order they were first proposed.
    pub fn all(&self) -> impl Iterator<Item = &Learning> {
        self.entries.iter()
    }

    /// How many are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Finds one by identifier.
    #[must_use]
    pub fn get(&self, id: &LearningId) -> Option<&Learning> {
        self.entries.iter().find(|learning| learning.id == *id)
    }

    /// The learnings an agent's next run should be given, oldest first.
    ///
    /// Confirmed ones lead, because they have earned their place, and a run that has to skim is
    /// better off skimming the provisional tail.
    #[must_use]
    pub fn active_for(&self, agent: &AgentName) -> Vec<&Learning> {
        let mut active: Vec<&Learning> = self
            .entries
            .iter()
            .filter(|learning| learning.agent == *agent && learning.is_active())
            .collect();

        active.sort_by(|left, right| {
            // Only the confirmed/provisional split and age matter: everything here is active by
            // construction, so comparing on that would be a comparison that can never differ.
            (right.state == State::Confirmed)
                .cmp(&(left.state == State::Confirmed))
                .then_with(|| left.first_at.cmp(&right.first_at))
        });
        active
    }

    /// Takes up a proposal, or explains why it was not taken up.
    pub fn propose(&mut self, proposal: &Proposal) -> Uptake {
        if !proposal.is_well_formed() {
            return Uptake::Malformed;
        }

        // Screened before anything else looks at it, and before it can match an existing learning.
        // A rediscovery of something that should never have been stored is still something that
        // should never have been stored.
        if let Err(reason) = crate::learning::screen(&proposal.text) {
            return Uptake::Unacceptable(reason);
        }

        let text = proposal.text.trim();
        let existing = self
            .entries
            .iter_mut()
            .find(|learning| learning.agent == proposal.agent && learning.matches(text));

        let Some(learning) = existing else {
            self.entries.push(Learning::from_proposal(proposal));
            return Uptake::Taken;
        };

        match learning.state {
            // A human said no. Saying it again does not change that, and counting it would let an
            // agent overturn a decision by repetition.
            State::Rejected => Uptake::Refused,

            // Being shown a learning and repeating it is not evidence of anything.
            State::Provisional | State::Confirmed => Uptake::Echo,

            State::Lapsed => {
                learning.proposals = learning.proposals.saturating_add(1);
                learning.last_at = proposal.at;
                // The rediscovery may be better worded, or rated differently now that the agent
                // has hit it twice. Take the newer text on the grounds that it was written with
                // more experience of the problem.
                learning.text.clear();
                learning.text.push_str(text);
                learning.impact = learning.impact.max(proposal.impact);

                if learning.proposals.saturating_sub(1) >= CONFIRM_AFTER {
                    learning.state = State::Confirmed;
                    learning.runs_left = 0;
                    Uptake::Confirmed
                } else {
                    learning.state = State::Provisional;
                    learning.runs_left = PROVISIONAL_RUNS;
                    Uptake::Rediscovered {
                        proposals: learning.proposals,
                    }
                }
            }
        }
    }

    /// Charges one run against `agent`'s provisional learnings, lapsing any that run out.
    ///
    /// Returns the learnings that lapsed, so the caller can say so rather than have advice
    /// disappear silently.
    pub fn charge_run(&mut self, agent: &AgentName) -> Vec<LearningId> {
        let mut lapsed = Vec::new();

        for learning in &mut self.entries {
            if learning.agent != *agent || learning.state != State::Provisional {
                continue;
            }

            learning.runs_left = learning.runs_left.saturating_sub(1);
            if learning.runs_left == 0 {
                learning.state = State::Lapsed;
                lapsed.push(learning.id.clone());
            }
        }

        lapsed
    }

    /// Marks a learning permanent, as a human confirming what they have read.
    ///
    /// The ordinary path to [`State::Confirmed`] is rediscovery; this is the shortcut for an
    /// operator who already knows the insight is right and does not want to wait for the factory
    /// to work it out twice more.
    pub fn confirm(&mut self, id: &LearningId, at: Timestamp) -> bool {
        self.with(id, |learning| {
            learning.state = State::Confirmed;
            learning.runs_left = 0;
            learning.last_at = at;
        })
    }

    /// Refuses a learning for good.
    ///
    /// This is the revocation path, and the reason applying immediately is defensible: when a
    /// learning turns out to be wrong there is one place to go and one thing to press.
    pub fn reject(&mut self, id: &LearningId, at: Timestamp) -> bool {
        self.with(id, |learning| {
            learning.state = State::Rejected;
            learning.runs_left = 0;
            learning.last_at = at;
        })
    }

    /// Applies a change to one learning, reporting whether it was there.
    fn with(&mut self, id: &LearningId, change: impl FnOnce(&mut Learning)) -> bool {
        match self.entries.iter_mut().find(|learning| learning.id == *id) {
            Some(learning) => {
                change(learning);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rfc3339: &str) -> Timestamp {
        rfc3339.parse().expect("valid timestamp")
    }

    fn proposal(text: &str) -> Proposal {
        Proposal::new(
            "reviewer".into(),
            text,
            Impact::Medium,
            at("2026-09-16T10:00:00Z"),
        )
    }

    fn reviewer() -> AgentName {
        "reviewer".into()
    }

    /// Runs an agent enough times to lapse everything provisional.
    fn lapse(learnings: &mut Learnings) {
        for _ in 0..PROVISIONAL_RUNS {
            learnings.charge_run(&reviewer());
        }
    }

    #[test]
    fn a_new_learning_applies_from_the_next_run() {
        // The whole point of not having an approval queue: value arrives immediately.
        let mut learnings = Learnings::new();

        assert_eq!(
            learnings.propose(&proposal("use ripgrep here")),
            Uptake::Taken
        );
        assert_eq!(learnings.active_for(&reviewer()).len(), 1);
    }

    #[test]
    fn repeating_advice_you_were_just_given_is_not_evidence() {
        // This is the subtlety that makes rediscovery meaningful. A learning being shown to an
        // agent contaminates the signal: without this, one fluke confirms itself in three runs.
        let mut learnings = Learnings::new();
        learnings.propose(&proposal("use ripgrep here"));

        assert_eq!(
            learnings.propose(&proposal("use ripgrep here")),
            Uptake::Echo
        );
        assert_eq!(
            learnings.propose(&proposal("Use ripgrep here.")),
            Uptake::Echo
        );
        assert_eq!(learnings.all().next().expect("one").proposals, 1);
    }

    #[test]
    fn a_learning_lapses_after_its_allotted_runs() {
        let mut learnings = Learnings::new();
        learnings.propose(&proposal("use ripgrep here"));

        for _ in 0..PROVISIONAL_RUNS - 1 {
            assert!(learnings.charge_run(&reviewer()).is_empty());
        }
        let lapsed = learnings.charge_run(&reviewer());

        assert_eq!(lapsed.len(), 1, "the last run should retire it");
        assert!(learnings.active_for(&reviewer()).is_empty());
        assert_eq!(learnings.all().next().expect("one").state, State::Lapsed);
    }

    #[test]
    fn rediscovering_a_lapsed_learning_counts_and_revives_it() {
        let mut learnings = Learnings::new();
        learnings.propose(&proposal("use ripgrep here"));
        lapse(&mut learnings);

        assert_eq!(
            learnings.propose(&proposal("use ripgrep here")),
            Uptake::Rediscovered { proposals: 2 }
        );
        assert_eq!(learnings.active_for(&reviewer()).len(), 1);
        assert_eq!(
            learnings.all().next().expect("one").runs_left,
            PROVISIONAL_RUNS,
            "a rediscovery earns a full second life"
        );
    }

    #[test]
    fn three_independent_rediscoveries_make_a_learning_permanent() {
        // Evidence rather than self-assessment: the agent keeps arriving at the same conclusion
        // without being told it, which is the only signal here that is not the agent's own claim
        // about its own work.
        let mut learnings = Learnings::new();
        learnings.propose(&proposal("the token expires every thirty days"));

        lapse(&mut learnings);
        assert_eq!(
            learnings.propose(&proposal("the token expires every thirty days")),
            Uptake::Rediscovered { proposals: 2 }
        );

        lapse(&mut learnings);
        assert_eq!(
            learnings.propose(&proposal("the token expires every thirty days")),
            Uptake::Rediscovered { proposals: 3 }
        );

        lapse(&mut learnings);
        assert_eq!(
            learnings.propose(&proposal("the token expires every thirty days")),
            Uptake::Confirmed
        );

        let learning = learnings.all().next().expect("one");
        assert_eq!(learning.state, State::Confirmed);
        assert_eq!(
            learning.proposals,
            CONFIRM_AFTER + 1,
            "the original discovery plus three rediscoveries"
        );
    }

    #[test]
    fn a_confirmed_learning_never_lapses() {
        let mut learnings = Learnings::new();
        learnings.propose(&proposal("the token expires every thirty days"));
        lapse(&mut learnings);
        learnings.propose(&proposal("the token expires every thirty days"));
        lapse(&mut learnings);
        learnings.propose(&proposal("the token expires every thirty days"));
        lapse(&mut learnings);
        learnings.propose(&proposal("the token expires every thirty days"));

        for _ in 0..PROVISIONAL_RUNS * 3 {
            learnings.charge_run(&reviewer());
        }

        assert_eq!(learnings.active_for(&reviewer()).len(), 1);
    }

    #[test]
    fn a_rejected_learning_cannot_be_reinstated_by_repetition() {
        // Otherwise an agent overturns a human decision simply by being persistent, which is the
        // one direction this system must not run in.
        let mut learnings = Learnings::new();
        learnings.propose(&proposal("skip the integration tests, they are flaky"));
        let id = learnings.all().next().expect("one").id.clone();

        assert!(learnings.reject(&id, at("2026-09-16T11:00:00Z")));

        for _ in 0..5 {
            assert_eq!(
                learnings.propose(&proposal("skip the integration tests, they are flaky")),
                Uptake::Refused
            );
        }
        assert!(learnings.active_for(&reviewer()).is_empty());
    }

    #[test]
    fn a_human_can_confirm_without_waiting_for_the_factory_to_agree() {
        let mut learnings = Learnings::new();
        learnings.propose(&proposal("use ripgrep here"));
        let id = learnings.all().next().expect("one").id.clone();

        assert!(learnings.confirm(&id, at("2026-09-16T11:00:00Z")));
        lapse(&mut learnings);

        assert_eq!(learnings.active_for(&reviewer()).len(), 1);
    }

    #[test]
    fn learnings_do_not_cross_agents() {
        let mut learnings = Learnings::new();
        learnings.propose(&proposal("use ripgrep here"));
        learnings.propose(&Proposal::new(
            "developer".into(),
            "use ripgrep here",
            Impact::Low,
            at("2026-09-16T10:00:00Z"),
        ));

        assert_eq!(learnings.active_for(&reviewer()).len(), 1);
        assert_eq!(learnings.active_for(&"developer".into()).len(), 1);
        assert_eq!(learnings.len(), 2, "same words, two separate insights");
    }

    #[test]
    fn one_agents_runs_do_not_age_anothers_learnings() {
        let mut learnings = Learnings::new();
        learnings.propose(&proposal("use ripgrep here"));

        for _ in 0..PROVISIONAL_RUNS * 2 {
            learnings.charge_run(&"developer".into());
        }

        assert_eq!(
            learnings.active_for(&reviewer()).len(),
            1,
            "a busy neighbour must not retire a quiet agent's advice"
        );
    }

    #[test]
    fn a_rediscovery_takes_the_newer_wording_and_the_higher_rating() {
        // The second time round the agent has more experience of the problem, so its wording is
        // likely better. Impact only ratchets up: a learning that turned out to matter more is
        // more interesting than one that was downgraded.
        let mut learnings = Learnings::new();
        learnings.propose(&Proposal::new(
            "reviewer".into(),
            "the token expires every thirty days",
            Impact::Low,
            at("2026-09-16T10:00:00Z"),
        ));
        lapse(&mut learnings);

        learnings.propose(&Proposal::new(
            "reviewer".into(),
            "the token expires every thirty days, refresh before publishing",
            Impact::High,
            at("2026-09-20T10:00:00Z"),
        ));

        let learning = learnings.all().next().expect("one");
        assert!(learning.text.contains("refresh before publishing"));
        assert_eq!(learning.impact, Impact::High);
        assert_eq!(learning.last_at, at("2026-09-20T10:00:00Z"));
    }

    #[test]
    fn a_malformed_proposal_is_refused_without_creating_anything() {
        let mut learnings = Learnings::new();

        assert_eq!(learnings.propose(&proposal("  ")), Uptake::Malformed);
        assert_eq!(
            learnings.propose(&proposal(&"x".repeat(super::super::MAX_TEXT + 1))),
            Uptake::Malformed
        );
        assert!(learnings.is_empty());
    }

    #[test]
    fn confirmed_learnings_are_offered_before_provisional_ones() {
        let mut learnings = Learnings::new();
        learnings.propose(&proposal("provisional advice about searching"));
        learnings.propose(&proposal("the token expires every thirty days"));

        let id = learnings
            .all()
            .find(|learning| learning.text.contains("token"))
            .expect("second")
            .id
            .clone();
        learnings.confirm(&id, at("2026-09-16T11:00:00Z"));

        let active = learnings.active_for(&reviewer());
        assert_eq!(active[0].state, State::Confirmed);
    }

    #[test]
    fn acting_on_a_learning_that_is_not_there_reports_so() {
        let mut learnings = Learnings::new();
        let ghost = LearningId::generate();

        assert!(!learnings.confirm(&ghost, at("2026-09-16T11:00:00Z")));
        assert!(!learnings.reject(&ghost, at("2026-09-16T11:00:00Z")));
    }
}
