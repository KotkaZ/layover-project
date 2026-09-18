//! What a run is told, and in what order.
//!
//! # Why the order is a decision and not a detail
//!
//! A run may be handed five different things: the agent's own instructions, its memory, what
//! earlier runs of it learned, a note explaining that this attempt follows an interrupted one, and
//! the message that woke it. Models weight the beginning and the end of a context differently, and
//! whatever arrives last reads as *the current instruction*.
//!
//! So the order is:
//!
//! ```text
//! 1. Identity        who you are -- from the Tower, never asserted by the agent
//! 2. Instructions    the composed prompt, @include resolved
//! 3. Memory          what this agent wrote down for itself, tail-capped
//! 4. Brief           what earlier runs learned, and how to ask for help
//! 5. Handover        only when this run follows an interrupted one, or a human steer
//! 6. Flight body     the message that woke it -- last
//! ```
//!
//! The body is last because it is the instruction; everything above is context for carrying it
//! out. The handover sits immediately above the body because it frames *this attempt* — "you are
//! continuing work that did not finish" only means anything next to what the work is.
//!
//! Learnings go above the handover rather than below, so a recovery instruction is never buried
//! under twenty-five lines of accumulated advice.
//!
//! # Why this is a pure function
//!
//! Composing what a run is told and *starting* a run are different problems, and only the second
//! needs a process. Keeping them apart means the thing most likely to be wrong — the text an agent
//! actually receives — can be read, diffed and tested without spawning anything, and
//! `layover prompt` can show it to a human before it costs money.

use std::fmt::Write as _;

use crate::agent::AgentName;

/// How much of an agent's memory is injected.
///
/// Memory is injected rather than fetched because an agent that forgets to call for it simply has
/// no memory, and nothing anywhere would report that. Silent failure is the thing this project
/// exists to avoid.
///
/// But memory grows without bound and the payload it joins already runs to tens of kilobytes, so
/// what arrives is the **tail** — the most recent thing the agent wrote down. When it is cut, the
/// text says so, because an agent that knows it is seeing an excerpt can go and read the rest.
pub const MAX_MEMORY: usize = 4_096;

/// Everything one run is told.
///
/// Borrowed rather than owned: every part of this already exists somewhere the Tower holds, and
/// copying a 34 KB prompt to build a struct that immediately concatenates it is wasted work.
#[derive(Debug, Clone, Copy)]
pub struct Run<'a> {
    /// Which agent this is. Comes from the Tower's own record, never from the agent.
    pub agent: &'a AgentName,
    /// The agent's composed instructions, with `@include` already resolved.
    pub instructions: &'a str,
    /// What the agent wrote down for itself, if anything.
    pub memory: Option<&'a str>,
    /// Learnings and the help instructions, from [`crate::brief::brief`].
    pub brief: &'a str,
    /// Why this run follows another, from [`crate::handover::Handover::brief`]. Empty for an
    /// ordinary dispatch.
    pub handover: Option<&'a str>,
    /// The message that woke this agent.
    pub body: &'a str,
}

/// Renders the payload a run receives on stdin.
///
/// This is the whole of what the process is told. There is no second channel: a runner that takes
/// a file gets this same text written to a path, and one that does not gets it on stdin.
#[must_use]
pub fn compose(run: &Run<'_>) -> String {
    let mut out = String::with_capacity(
        run.instructions.len() + run.brief.len() + run.body.len() + MAX_MEMORY + 256,
    );

    write_identity(&mut out, run.agent);
    write_section(&mut out, run.instructions);

    if let Some(memory) = run.memory {
        write_memory(&mut out, memory);
    }

    write_section(&mut out, run.brief);

    if let Some(handover) = run.handover {
        write_section(&mut out, handover);
    }

    write_body(&mut out, run.body);
    out
}

/// States who the agent is.
///
/// Three words, and without them every prompt file has to hard-code its own agent's name — which
/// drifts the first time somebody renames one. It also has to come from here rather than from the
/// agent: identity the agent asserts is identity an agent can lie about.
fn write_identity(out: &mut String, agent: &AgentName) {
    let _ = writeln!(out, "You are `{agent}`.\n");
}

/// Appends a block, keeping exactly one blank line between sections.
fn write_section(out: &mut String, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }

    out.push_str(text);
    out.push_str("\n\n");
}

/// Appends the agent's own memory, cut to the most recent [`MAX_MEMORY`] bytes.
fn write_memory(out: &mut String, memory: &str) {
    let memory = memory.trim();
    if memory.is_empty() {
        return;
    }

    out.push_str("== WHAT YOU WROTE DOWN LAST TIME ==\n");
    out.push_str(
        "Runs are a clean slate, so this is everything you remember. It is what earlier runs of \
         you chose to record, and nothing else carried over.\n\n",
    );

    if memory.len() <= MAX_MEMORY {
        out.push_str(memory);
        out.push_str("\n\n");
        return;
    }

    // Cut from the front: the end of the file is the most recent thing written, and a memory that
    // keeps only its oldest entries gets less useful the longer an agent runs.
    let mut start = memory.len() - MAX_MEMORY;
    while start < memory.len() && !memory.is_char_boundary(start) {
        start += 1;
    }

    out.push_str(
        "[This is the most recent part of your memory. It is longer than fits here — call \
         `layover_memory_read` for the whole file.]\n\n",
    );
    out.push_str(memory[start..].trim_start());
    out.push_str("\n\n");
}

/// Appends the message that woke the agent.
fn write_body(out: &mut String, body: &str) {
    out.push_str("== WHAT YOU HAVE BEEN ASKED TO DO ==\n\n");
    out.push_str(body.trim());
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> AgentName {
        AgentName::new("tester")
    }

    fn minimal<'a>(agent: &'a AgentName, body: &'a str) -> Run<'a> {
        Run {
            agent,
            instructions: "Run the suite and report what failed.",
            memory: None,
            brief: "",
            handover: None,
            body,
        }
    }

    /// The order *is* the decision, so it is pinned rather than left to whoever edits next.
    #[test]
    fn the_sections_arrive_in_the_settled_order() {
        let name = agent();
        let run = Run {
            agent: &name,
            instructions: "INSTRUCTIONS",
            memory: Some("MEMORY"),
            brief: "BRIEF",
            handover: Some("HANDOVER"),
            body: "BODY",
        };

        let text = compose(&run);
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("missing {needle}"))
        };

        assert!(at("You are `tester`") < at("INSTRUCTIONS"));
        assert!(at("INSTRUCTIONS") < at("MEMORY"));
        assert!(at("MEMORY") < at("BRIEF"));
        assert!(
            at("BRIEF") < at("HANDOVER"),
            "a recovery instruction must not be buried under accumulated advice"
        );
        assert!(
            at("HANDOVER") < at("BODY"),
            "the message that woke the agent reads as the current instruction, so it goes last"
        );
    }

    #[test]
    fn the_body_is_the_last_thing_in_the_payload() {
        let name = agent();
        let text = compose(&minimal(&name, "Fix the retry policy."));

        assert!(text.trim_end().ends_with("Fix the retry policy."), "{text}");
    }

    #[test]
    fn identity_comes_from_the_tower_and_is_always_present() {
        let name = agent();
        assert!(compose(&minimal(&name, "go")).starts_with("You are `tester`."));
    }

    #[test]
    fn absent_sections_leave_no_hole() {
        let name = agent();
        let text = compose(&minimal(&name, "go"));

        assert!(!text.contains("WHAT YOU WROTE DOWN"), "{text}");
        assert!(
            !text.contains("\n\n\n"),
            "blank lines should not stack: {text:?}"
        );
    }

    #[test]
    fn memory_shorter_than_the_cap_arrives_whole_and_says_nothing_about_cutting() {
        let name = agent();
        let run = Run {
            memory: Some("The e2e suite needs VPN. Ask before assuming a failure is real."),
            ..minimal(&name, "go")
        };

        let text = compose(&run);
        assert!(text.contains("needs VPN"), "{text}");
        assert!(!text.contains("most recent part"), "{text}");
    }

    #[test]
    fn over_long_memory_keeps_the_end_not_the_beginning() {
        // The end is the most recent thing written. A memory that kept only its oldest entries
        // would get less useful the longer an agent ran, which is the opposite of the point.
        let name = agent();
        let memory = format!("OLDEST{}NEWEST", "x".repeat(MAX_MEMORY * 2));
        let run = Run {
            memory: Some(&memory),
            ..minimal(&name, "go")
        };

        let text = compose(&run);
        assert!(text.contains("NEWEST"), "the recent end was dropped");
        assert!(!text.contains("OLDEST"), "the old end was kept");
    }

    #[test]
    fn a_cut_memory_says_so_and_says_where_the_rest_is() {
        let name = agent();
        let memory = "y".repeat(MAX_MEMORY * 2);
        let run = Run {
            memory: Some(&memory),
            ..minimal(&name, "go")
        };

        let text = compose(&run);
        assert!(text.contains("longer than fits here"), "{text}");
        assert!(
            text.contains("layover_memory_read"),
            "an agent told it has more should be told how to get it: {text}"
        );
    }

    #[test]
    fn cutting_memory_never_splits_a_character() {
        let name = agent();
        // Multi-byte throughout, so a naive byte offset lands mid-character.
        let memory = "é".repeat(MAX_MEMORY);
        let run = Run {
            memory: Some(&memory),
            ..minimal(&name, "go")
        };

        assert!(compose(&run).contains('é'));
    }

    #[test]
    fn an_ordinary_dispatch_carries_no_handover() {
        let name = agent();
        let text = compose(&minimal(&name, "go"));

        assert!(!text.contains("continuing"), "{text}");
    }

    #[test]
    fn empty_memory_is_the_same_as_no_memory() {
        let name = agent();
        let blank = Run {
            memory: Some("   \n  "),
            ..minimal(&name, "go")
        };

        assert_eq!(compose(&blank), compose(&minimal(&name, "go")));
    }
}
