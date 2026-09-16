//! The block of text a run is given beyond its own instructions.
//!
//! Three things go in, and each earns its place by answering a question the run cannot answer for
//! itself: *what have we learned that applies to you*, *what is already known so do not say it
//! again*, and *how do you tell us when you are stuck*.
//!
//! # Why this is generated rather than written into every prompt
//!
//! It changes per run. Learnings arrive, lapse and are revoked; the already-captured list depends
//! on what is live right now. Baking any of it into an agent's prompt file would make the file
//! wrong the moment the factory learned something, and nobody would notice.

use std::fmt::Write as _;

use crate::agent::AgentName;
use crate::help::Blocker;
use crate::learning::{Impact, Learning, Learnings, State};

/// How many learnings are given to one run at most.
///
/// A ceiling rather than a target. Everything here is prepended to a prompt that already runs to
/// tens of kilobytes, and an agent that skims a wall of advice follows none of it. Confirmed
/// learnings come first, so what is dropped is the least-established.
pub const MAX_INJECTED: usize = 25;

/// Builds the block for one run.
///
/// Empty when there is nothing to say and the agent cannot ask for help, so a factory that has
/// learned nothing adds nothing to its prompts.
#[must_use]
pub fn brief(agent: &AgentName, learnings: &Learnings, can_ask_for_help: bool) -> String {
    let mut out = String::new();

    let active = learnings.active_for(agent);
    write_learnings(&mut out, &active);
    write_proposing(&mut out, agent, &active);
    if can_ask_for_help {
        write_help(&mut out);
    }

    out
}

/// Lists what earlier runs of this agent worked out.
fn write_learnings(out: &mut String, active: &[&Learning]) {
    if active.is_empty() {
        return;
    }

    out.push_str("\n== WHAT EARLIER RUNS LEARNED ==\n");
    out.push_str(
        "Apply these. They came from runs of this agent, not from a person, so treat them as \
         strong priors rather than instructions: if one contradicts what you can see in front of \
         you, believe your own eyes and say so.\n\n",
    );

    for (index, learning) in active.iter().take(MAX_INJECTED).enumerate() {
        let standing = match learning.state {
            State::Confirmed => "established",
            _ => "provisional",
        };
        let _ = writeln!(out, "{}. [{standing}] {}", index + 1, learning.text);
    }
}

/// Explains how to add one, and what is already known.
fn write_proposing(out: &mut String, agent: &AgentName, active: &[&Learning]) {
    out.push_str("\n== PROPOSE A LEARNING (optional) ==\n");
    out.push_str(
        "If this run turns up something durable and reusable that would make future runs of this \
         agent better — a gotcha, a reliable command, a convention, a mistake to avoid — record \
         it with the `layover_learn` tool. Run-specific facts are not learnings; if you have \
         nothing genuinely new, say nothing, which is the common case.\n",
    );
    let _ = writeln!(
        out,
        "It applies to future runs of `{agent}` only, and it lapses unless later runs arrive at \
         it independently."
    );
    let _ = writeln!(
        out,
        "Rate the impact honestly: `{}` prevents a wrong result or a blocked run, `{}` improves \
         reliability, `{}` is polish. Most things are `{}`.",
        Impact::High,
        Impact::Medium,
        Impact::Low,
        Impact::Medium
    );

    if active.is_empty() {
        return;
    }

    // Suppressing duplicates here is not about tidiness. A learning you were just shown and then
    // repeat is an echo, and counting echoes would let a single fluke confirm itself.
    //
    // Referring back to the list rather than reprinting it: the two are identical by
    // construction, and this block is prepended to every run of this agent, so a second copy of
    // twenty-five learnings is a recurring cost for no added meaning. Lapsed learnings are
    // deliberately absent from both — re-proposing one is exactly the rediscovery that confirms
    // it.
    out.push_str(
        "\nDo not propose anything already listed above, in any wording. Repeating advice you \
         were just given is not a rediscovery, and it is ignored. If one of them is wrong or \
         incomplete, do not restate it — propose a short, explicit correction that says what \
         changes.\n",
    );
}

/// Explains how to ask a human for something.
fn write_help(out: &mut String) {
    out.push_str("\n== ASKING FOR HELP ==\n");
    out.push_str(
        "If something is genuinely in your way, call the `layover_help` tool. It reaches a person; \
         nothing else here does.\n\n",
    );
    out.push_str(
        "- Prefer progress over stalling. Proceed on the most likely reading and say what you \
         assumed. Ask only when you cannot move forward, and never speculate past your evidence.\n",
    );
    out.push_str(
        "- Do not work around a denied permission. A refusal you route around is a refusal \
         nobody gets to reconsider. Report it and stop.\n",
    );
    out.push_str(
        "- Ask once per blocker. A recurring problem is one request with a count, not one per \
         run.\n",
    );
    out.push_str(
        "- Say whether it stopped you or merely limited you. Finishing the work while being \
         unable to check one thing is worth reporting and is not an outage.\n",
    );
    out.push_str("\nCategories:\n");
    for blocker in Blocker::ALL {
        let _ = writeln!(out, "- `{blocker}` — {}", blocker.describe());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::Proposal;
    use jiff::Timestamp;

    fn at() -> Timestamp {
        "2026-09-16T10:00:00Z".parse().expect("valid timestamp")
    }

    fn reviewer() -> AgentName {
        "reviewer".into()
    }

    fn with(texts: &[&str]) -> Learnings {
        let mut learnings = Learnings::new();
        for text in texts {
            learnings.propose(&Proposal::new(reviewer(), *text, Impact::Medium, at()));
        }
        learnings
    }

    #[test]
    fn a_factory_that_has_learned_nothing_still_explains_both_channels() {
        let brief = brief(&reviewer(), &Learnings::new(), true);

        assert!(!brief.contains("WHAT EARLIER RUNS LEARNED"));
        assert!(brief.contains("PROPOSE A LEARNING"));
        assert!(brief.contains("ASKING FOR HELP"));
    }

    #[test]
    fn an_agent_that_cannot_ask_for_help_is_not_told_how() {
        let brief = brief(&reviewer(), &Learnings::new(), false);

        assert!(!brief.contains("ASKING FOR HELP"));
        assert!(!brief.contains("layover_help"));
    }

    #[test]
    fn learnings_are_listed_with_how_well_established_they_are() {
        // A run should weigh advice that has been rediscovered three times differently from
        // something one earlier run thought once.
        let mut learnings = with(&["prefer ripgrep when searching the tree"]);
        let id = learnings.all().next().expect("one").id.clone();
        learnings.confirm(&id, at());
        learnings.propose(&Proposal::new(
            reviewer(),
            "the build needs the vcvars script sourced first",
            Impact::Medium,
            at(),
        ));

        let brief = brief(&reviewer(), &learnings, true);

        assert!(brief.contains("[established] prefer ripgrep"), "{brief}");
        assert!(brief.contains("[provisional] the build needs"), "{brief}");
    }

    #[test]
    fn a_run_is_told_to_believe_its_own_eyes_over_a_learning() {
        // These come from an agent, not a person. A run that follows a stale learning past
        // contrary evidence in front of it is worse than one that never had the learning.
        let brief = brief(&reviewer(), &with(&["the token expires monthly"]), true);

        assert!(brief.contains("believe your own eyes"));
    }

    #[test]
    fn active_learnings_are_also_listed_as_things_not_to_propose_again() {
        // The echo-suppression that protects the confirmation signal.
        let brief = brief(&reviewer(), &with(&["the token expires monthly"]), true);

        assert!(brief.contains("not a rediscovery"));
        assert_eq!(
            brief.matches("the token expires monthly").count(),
            1,
            "printed once and referred back to; a second copy is a recurring cost for no meaning"
        );
    }

    #[test]
    fn a_correction_is_invited_instead_of_a_restatement() {
        let brief = brief(&reviewer(), &with(&["the token expires monthly"]), true);

        assert!(brief.contains("explicit correction"));
    }

    #[test]
    fn the_number_of_learnings_given_to_a_run_is_capped() {
        // Everything here is prepended to a prompt that already runs to tens of kilobytes, and an
        // agent that skims a wall of advice follows none of it.
        //
        // The fixture needs genuinely different vocabulary, not a counter: short tokens are
        // dropped before comparison, so thirty-five sentences differing only by a number are one
        // insight as far as the matcher is concerned — which it was right about.
        let subjects = [
            "workspace",
            "manifest",
            "registry",
            "pipeline",
            "artefact",
            "checkout",
            "telemetry",
            "migration",
            "container",
            "signature",
            "scheduler",
            "changelog",
            "dependency",
            "benchmark",
            "clipboard",
            "renderer",
            "validator",
            "throttle",
            "sandbox",
            "gateway",
            "resolver",
            "formatter",
            "profiler",
            "snapshot",
            "installer",
            "extension",
            "descriptor",
            "publisher",
            "transcript",
            "allocator",
            "namespace",
            "predicate",
            "serialiser",
            "reconciler",
            "diagnostic",
        ];
        let texts: Vec<String> = subjects
            .iter()
            .map(|subject| format!("the {subject} needs careful handling before publishing"))
            .collect();
        let learnings = with(&texts.iter().map(String::as_str).collect::<Vec<_>>());

        assert!(
            learnings.len() > MAX_INJECTED,
            "the fixture must overflow the cap, got {}",
            learnings.len()
        );

        let brief = brief(&reviewer(), &learnings, true);
        assert_eq!(
            brief.matches("[provisional]").count(),
            MAX_INJECTED,
            "more than the cap reached the run"
        );
    }

    #[test]
    fn another_agents_learnings_do_not_appear() {
        let mut learnings = with(&["reviewer-only advice about the tree"]);
        learnings.propose(&Proposal::new(
            "developer".into(),
            "developer-only advice about the build",
            Impact::Medium,
            at(),
        ));

        let brief = brief(&reviewer(), &learnings, true);

        assert!(brief.contains("reviewer-only"));
        assert!(!brief.contains("developer-only"));
    }

    #[test]
    fn the_help_protocol_tells_an_agent_not_to_route_around_a_refusal() {
        // The behaviour worth preserving from the prototype this is modelled on: its agents
        // reliably reported denied permissions instead of finding another way through.
        let brief = brief(&reviewer(), &Learnings::new(), true);

        assert!(brief.contains("Do not work around a denied permission"));
        assert!(brief.contains("Ask once per blocker"));
        assert!(brief.contains("Prefer progress over stalling"));
    }

    #[test]
    fn every_blocker_category_is_offered_with_an_explanation() {
        let brief = brief(&reviewer(), &Learnings::new(), true);

        for blocker in Blocker::ALL {
            assert!(brief.contains(blocker.describe()), "{blocker} unexplained");
        }
    }
}
