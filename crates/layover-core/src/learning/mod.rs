//! Agents proposing how to do better next time.
//!
//! An agent that discovers something durable — a gotcha, a reliable command, a convention — can
//! write it down for the agents that come after it. That is the whole feature, and the hard part
//! is not capturing them.
//!
//! # Why there is no approval queue
//!
//! The obvious design puts a human between a proposal and its use. A sibling project built
//! exactly that, carefully: a proposal format, duplicate detection, impact ratings, a review
//! endpoint and a dashboard queue. After 22 days of real operation it held **88 learnings, every
//! one still pending, none ever approved** — and because only approved learnings were injected,
//! not one had ever reached a run.
//!
//! That is not a discipline failure. Approving buys a diffuse future benefit, rejecting buys
//! nothing, and ignoring costs nothing today, so the rational act is always "later". A gate whose
//! default action is free will be defaulted forever.
//!
//! So a learning here applies immediately and **expires** instead. A wrong one decays rather than
//! compounding, and the human reviews by exception — which is possible because the run history
//! records which learnings were live for each run, so "what was it told when it did that?" is an
//! answerable question.
//!
//! # Why re-proposal is the confirmation signal, and why echoes do not count
//!
//! A learning that is genuinely true gets rediscovered. One that was a fluke does not. Counting
//! independent rediscoveries is therefore evidence, unlike an impact rating, which is the agent's
//! own claim about its own work — the thing the architecture says not to trust.
//!
//! The subtlety is that a learning being *shown* to an agent contaminates the signal: re-proposing
//! something you were just reminded of is an echo, not a rediscovery. So duplicates are suppressed
//! while a learning is active — the same behaviour the sibling project needed, for the opposite
//! reason — and only a proposal arriving while the learning is **lapsed** counts towards
//! confirmation.

use std::fmt;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::agent::AgentName;

mod ledger;

pub use ledger::{Learnings, Uptake};

/// How many independent rediscoveries make a learning permanent.
///
/// A rediscovery is an arrival *after* the learning has lapsed — the first proposal is a
/// discovery, not a rediscovery, and does not count towards this. Reaching the bar therefore
/// takes roughly `CONFIRM_AFTER * PROVISIONAL_RUNS` of the agent's runs, which is deliberate:
/// permanence is the one state nothing expires out of, so it should be expensive.
pub const CONFIRM_AFTER: u32 = 3;

/// How many of an agent's runs a provisional learning survives.
///
/// Counted in runs rather than days deliberately: an hourly pipeline and a manual one should not
/// share a clock. This is also the window in which a wrong learning can do damage, which is the
/// number to lower if that ever stops feeling acceptable.
pub const PROVISIONAL_RUNS: u32 = 20;

/// Longest a learning may be, in characters.
///
/// Injected into every run of its agent, so length is a recurring cost. The limit also pushes
/// towards one idea per learning, which is what makes them individually revocable.
pub const MAX_TEXT: usize = 400;

/// Identifier of a learning.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct LearningId(String);

impl LearningId {
    /// Mints a new identifier.
    #[must_use]
    pub fn generate() -> Self {
        Self(format!("lrn_{}", Ulid::new()))
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LearningId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How much applying a learning would change a future run.
///
/// Self-assessed, and therefore used for *display and triage only* — never to decide whether a
/// learning applies. An agent rating its own work is exactly the claim the architecture says not
/// to trust with anything load-bearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Impact {
    /// Minor convenience or polish.
    Low,
    /// Materially improves reliability or accuracy.
    Medium,
    /// Prevents a wrong result, a shipped defect, or a blocked run.
    High,
}

impl Impact {
    /// The identifier used in JSON.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Parses a slug.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        [Self::Low, Self::Medium, Self::High]
            .into_iter()
            .find(|impact| impact.slug() == slug)
    }
}

impl fmt::Display for Impact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

/// What an agent submitted.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Proposal {
    /// Who proposed it.
    pub agent: AgentName,
    /// The insight, in one or two sentences.
    pub text: String,
    /// How much the agent thinks it matters.
    pub impact: Impact,
    /// When it was proposed.
    pub at: Timestamp,
}

impl Proposal {
    /// Records a proposal.
    #[must_use]
    pub fn new(agent: AgentName, text: impl Into<String>, impact: Impact, at: Timestamp) -> Self {
        Self {
            agent,
            text: text.into(),
            impact,
            at,
        }
    }

    /// Returns `true` when the text is usable.
    ///
    /// Only length and emptiness. Judging whether an insight is *good* is not something a
    /// validator can do, and pretending otherwise would reject useful things.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        let trimmed = self.text.trim();
        !trimmed.is_empty() && trimmed.chars().count() <= MAX_TEXT
    }
}

/// Where a learning is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Applies now, and will lapse unless it is rediscovered.
    Provisional,
    /// Rediscovered enough times to be treated as real. Applies indefinitely.
    Confirmed,
    /// Ran out of runs without being rediscovered. Not applied, but remembered, so that a later
    /// rediscovery can be recognised as one.
    Lapsed,
    /// A human said no. Never applied and never counted again.
    Rejected,
}

impl State {
    /// Returns `true` when a learning in this state is given to runs.
    #[must_use]
    pub fn is_active(self) -> bool {
        matches!(self, Self::Provisional | Self::Confirmed)
    }

    /// The identifier used in JSON.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Provisional => "provisional",
            Self::Confirmed => "confirmed",
            Self::Lapsed => "lapsed",
            Self::Rejected => "rejected",
        }
    }

    /// Parses a slug.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        [
            Self::Provisional,
            Self::Confirmed,
            Self::Lapsed,
            Self::Rejected,
        ]
        .into_iter()
        .find(|state| state.slug() == slug)
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

/// A proposal that has been taken up, with its history.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Learning {
    /// Identifier.
    pub id: LearningId,
    /// The agent this applies to. Learnings do not cross agents.
    pub agent: AgentName,
    /// The insight.
    pub text: String,
    /// The agent's own rating, for triage.
    pub impact: Impact,
    /// Where it is in its life.
    pub state: State,
    /// How many times this insight has been proposed: one for the original discovery, plus one
    /// for each independent rediscovery after a lapse. Echoes never count.
    pub proposals: u32,
    /// Runs left before it lapses. Meaningless unless [`State::Provisional`].
    pub runs_left: u32,
    /// When it was first proposed.
    pub first_at: Timestamp,
    /// When it was most recently proposed.
    pub last_at: Timestamp,
}

impl Learning {
    /// Takes up a proposal for the first time.
    #[must_use]
    pub fn from_proposal(proposal: &Proposal) -> Self {
        Self {
            id: LearningId::generate(),
            agent: proposal.agent.clone(),
            text: proposal.text.trim().to_owned(),
            impact: proposal.impact,
            state: State::Provisional,
            proposals: 1,
            runs_left: PROVISIONAL_RUNS,
            first_at: proposal.at,
            last_at: proposal.at,
        }
    }

    /// Returns `true` when this learning is currently given to runs.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    /// Returns `true` when this says the same thing as `text`.
    #[must_use]
    pub fn matches(&self, text: &str) -> bool {
        says_the_same_thing(&self.text, text)
    }
}

/// Decides whether two pieces of prose say the same thing.
///
/// Overall word overlap turns out to be the wrong measure, because real learnings share sentence
/// frames. "The workspace needs careful handling before publishing" and "the manifest needs
/// careful handling before publishing" overlap by five words out of seven while being entirely
/// different claims; the difference lives in the one word the frame does not supply.
///
/// So the test is **containment**, not overlap. A rediscovery phrased with an extra clause is a
/// *superset* of the original — every word of the shorter appears in the longer. Two different
/// insights each carry a word the other lacks, however much boilerplate they share.
///
/// Two guards keep that honest. The shorter text must carry enough words to mean something, or
/// "use ripgrep" would match every sentence that happens to contain both words. And the longer
/// must not be wildly longer, because a statement several times more specific is a different,
/// narrower claim rather than the same one restated — which is exactly what a refinement is.
///
/// Near-identical rewordings are caught separately by a high overlap threshold, since those are
/// the same length and differ only in punctuation or a synonym.
///
/// Getting this wrong in either direction has a cost worth stating. Too strict and a rediscovery
/// is never recognised, so nothing is ever confirmed and every learning lapses forever — the
/// mechanism fails silently while appearing to work. Too loose and two insights merge, and one is
/// lost without trace.
#[must_use]
pub fn says_the_same_thing(left: &str, right: &str) -> bool {
    // A negation mismatch is disqualifying rather than one more differing word. Adding `not` to
    // an eight-word sentence still scores 0.83 similarity, comfortably over the rewording
    // threshold — so treating it as an ordinary token left "X is safe" and "X is not safe" as one
    // insight. Nothing about the shape of the two sentences can be allowed to outvote the fact
    // that one asserts the opposite of the other.
    if is_negated(left) != is_negated(right) {
        return false;
    }

    let (left, right) = (significant_words(left), significant_words(right));
    // Two texts with nothing substantive in them are not evidence of anything. Returning "equal"
    // here made every pair of short scraps one insight — "use rg now" and "go to bed" matched.
    if left.is_empty() || right.is_empty() {
        return false;
    }

    let shared = left.iter().filter(|word| right.contains(*word)).count();
    let (shorter, longer) = (left.len().min(right.len()), left.len().max(right.len()));

    let union = longer + shorter - shared;
    if union > 0 && precise(shared) / precise(union) >= REWORDING_THRESHOLD {
        return true;
    }

    shared == shorter
        && shorter >= MIN_WORDS_TO_CONTAIN
        && precise(longer) <= precise(shorter) * MOST_ELABORATION
}

/// Overlap at which two texts are taken to be the same thing reworded.
const REWORDING_THRESHOLD: f64 = 0.8;

/// Fewest significant words a text must have for containment to mean anything.
const MIN_WORDS_TO_CONTAIN: usize = 3;

/// How much longer an elaboration may be before it counts as a different, narrower claim.
const MOST_ELABORATION: f64 = 2.0;

/// Widens a small count for a ratio.
fn precise(count: usize) -> f64 {
    u32::try_from(count).map_or(f64::from(u32::MAX), f64::from)
}

/// Returns `true` when a text reverses the sense of what it says.
///
/// Deliberately generous about what counts. A false positive here makes two genuinely equivalent
/// learnings look different, which costs one spurious confirmation count. A false negative merges
/// a claim with its own correction, which loses the correction and keeps the wrong claim — so the
/// list includes words that only sometimes negate.
fn is_negated(text: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .any(|word| NEGATIONS.contains(&word.to_lowercase().as_str()))
}

/// Words taken to reverse a claim, at any length.
///
/// `t` is here because splitting on non-alphanumerics turns `don't` and `isn't` into `don`/`isn`
/// and `t`.
const NEGATIONS: [&str; 10] = [
    "not", "no", "nor", "never", "t", "cannot", "without", "unless", "neither", "none",
];

/// Reduces prose to the set of words worth comparing.
///
/// Tokens of three characters or fewer are dropped, because they are overwhelmingly articles and
/// prepositions and counting them makes every sentence resemble every other one. Numbers survive
/// that rule: `30` and `90` are under four characters, so "the token expires every 30 days" and
/// "every 90 days" were the same learning, and numeric facts are most of what a learning is.
fn significant_words(text: &str) -> Vec<String> {
    let mut words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|word| word.chars().count() > 3 || word.chars().any(|c| c.is_ascii_digit()))
        .collect();
    words.sort();
    words.dedup();
    words
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

    #[test]
    fn a_new_learning_starts_provisional_with_a_full_life() {
        let learning = Learning::from_proposal(&proposal("prefer ripgrep over findstr"));

        assert_eq!(learning.state, State::Provisional);
        assert!(learning.is_active(), "it applies from the first run");
        assert_eq!(learning.proposals, 1);
        assert_eq!(learning.runs_left, PROVISIONAL_RUNS);
    }

    #[test]
    fn only_provisional_and_confirmed_learnings_reach_a_run() {
        assert!(State::Provisional.is_active());
        assert!(State::Confirmed.is_active());
        assert!(!State::Lapsed.is_active());
        assert!(!State::Rejected.is_active());
    }

    #[test]
    fn rewording_and_punctuation_do_not_make_a_new_insight() {
        assert!(says_the_same_thing(
            "The ADO token expires every 30 days; refresh it before publishing.",
            "the ADO token expires every 30 days, refresh it before publishing"
        ));
    }

    #[test]
    fn a_sentence_with_one_clause_added_is_still_the_same_insight() {
        // The case that decides the threshold. If this does not match, a rediscovery worded even
        // slightly better is never recognised as one, nothing is ever confirmed, and every
        // learning lapses forever -- the mechanism fails silently and looks like it works.
        assert!(says_the_same_thing(
            "the token expires every thirty days",
            "the token expires every thirty days, refresh before publishing"
        ));
    }

    #[test]
    fn a_claim_and_its_negation_are_not_the_same_claim() {
        // The worst case this function can produce, and it was live. "not" is three characters,
        // so both sides reduced to the same word set and matched at similarity 1.0. The effect
        // in the ledger: an agent proposing the correction gets `Echo`, the correction is
        // dropped, and the dangerous learning stays active -- while across lapse cycles the two
        // contradictory phrasings count as independent rediscoveries of "the same insight" and
        // drive it to permanent.
        assert!(!says_the_same_thing(
            "the migration is safe to run during business hours",
            "the migration is not safe to run during business hours"
        ));
        assert!(!says_the_same_thing(
            "delete the old worktree before starting the next itinerary",
            "do not delete the old worktree before starting the next itinerary"
        ));
    }

    #[test]
    fn learnings_differing_only_in_a_number_are_different_learnings() {
        // Numeric facts are most of what a learning is, and digits were being filtered out for
        // being short.
        assert!(!says_the_same_thing(
            "the token expires every 30 days",
            "the token expires every 90 days"
        ));
    }

    #[test]
    fn two_texts_with_nothing_substantive_in_them_do_not_match() {
        // Returning "equal" for two empty word sets made every pair of short scraps one insight.
        assert!(!says_the_same_thing("use rg now", "go to bed"));
        assert!(!says_the_same_thing("", ""));
    }

    #[test]
    fn two_claims_sharing_a_sentence_frame_are_not_the_same_claim() {
        // The case that broke the first two attempts. These overlap by five words out of seven
        // while being entirely different facts; the difference lives in the one word the frame
        // does not supply.
        assert!(!says_the_same_thing(
            "the workspace needs careful handling before publishing",
            "the manifest needs careful handling before publishing"
        ));
    }

    #[test]
    fn genuinely_different_insights_stay_separate() {
        // A false match silently merges two insights and loses one, which is worse than an
        // occasional missed duplicate.
        assert!(!says_the_same_thing(
            "the ADO token expires every thirty days",
            "prefer ripgrep over findstr when searching the tree"
        ));
    }

    #[test]
    fn a_refinement_is_not_the_same_as_the_thing_it_refines() {
        assert!(!says_the_same_thing(
            "exclude the assistant bot from new activity",
            "exclude the assistant bot and the build service account from new activity, but only \
             when the commit tip is unchanged since the previous review cycle"
        ));
    }

    #[test]
    fn an_empty_or_overlong_proposal_is_not_well_formed() {
        assert!(!proposal("   ").is_well_formed());
        assert!(!proposal(&"x".repeat(MAX_TEXT + 1)).is_well_formed());
        assert!(proposal("something short and useful").is_well_formed());
    }

    #[test]
    fn impact_is_ordered_so_a_queue_can_be_triaged() {
        assert!(Impact::High > Impact::Medium);
        assert!(Impact::Medium > Impact::Low);
        for impact in [Impact::Low, Impact::Medium, Impact::High] {
            assert_eq!(Impact::from_slug(impact.slug()), Some(impact));
        }
    }

    #[test]
    fn states_round_trip() {
        for state in [
            State::Provisional,
            State::Confirmed,
            State::Lapsed,
            State::Rejected,
        ] {
            assert_eq!(State::from_slug(state.slug()), Some(state));
        }
        assert_eq!(State::from_slug("approved"), None);
    }

    #[test]
    fn a_learning_serialises_readably() {
        let line = serde_json::to_string(&Learning::from_proposal(&proposal("use ripgrep")))
            .expect("serialises");

        assert!(line.contains(r#""state":"provisional""#), "{line}");
        assert!(line.contains(r#""impact":"medium""#), "{line}");
        assert!(
            line.contains(r#""first_at":"2026-09-16T10:00:00Z""#),
            "{line}"
        );
    }
}
