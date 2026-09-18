//! The tools an agent may call, named once.
//!
//! # Why the registry is here rather than in the server
//!
//! Prompts tell agents to call these tools. Before this existed, the names in prompts, in the
//! book and in the code disagreed — eleven were documented and none were implemented — because
//! nothing could check. A name is only useful if the thing answering to it exists.
//!
//! Putting the list in the domain crate means `layover validate` can read a prompt, find every
//! tool it mentions, and refuse a factory that tells an agent to call something that is not there.
//! That check is worth more than it sounds: an agent instructed to use a tool it does not have
//! will improvise, and improvising is exactly what a factory is meant not to do unattended.
//!
//! # What is deliberately absent
//!
//! There is no `layover_spawn`. A `mode = "spawn"` route already opens one itinerary per flight,
//! and a tool that did the same would be a second permission model over the same graph — two
//! places to look when asking what an agent is allowed to start, which is one too many.

use std::fmt;

/// A tool an agent can call over MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tool {
    /// Send a flight to another agent. The only way work moves.
    Send,
    /// Ask who this agent may reach, and what each of them is for.
    Peers,
    /// Say what this run concluded.
    Report,
    /// Say what is in the way.
    Help,
    /// Read this agent's own notes.
    MemoryRead,
    /// Add to this agent's own notes.
    MemoryWrite,
    /// Propose something future runs of this agent should know.
    Learn,
    /// Add to the factory's shared memory.
    LogbookAppend,
    /// Ask what this run has left: its Hops, its Fuel.
    Status,
    /// Set work down to be picked up later.
    Wait,
}

impl Tool {
    /// Every tool, in a stable order.
    pub const ALL: [Self; 10] = [
        Self::Send,
        Self::Peers,
        Self::Report,
        Self::Help,
        Self::MemoryRead,
        Self::MemoryWrite,
        Self::Learn,
        Self::LogbookAppend,
        Self::Status,
        Self::Wait,
    ];

    /// The name an agent calls it by.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Send => "layover_send",
            Self::Peers => "layover_peers",
            Self::Report => "layover_report",
            Self::Help => "layover_help",
            Self::MemoryRead => "layover_memory_read",
            Self::MemoryWrite => "layover_memory_write",
            Self::Learn => "layover_learn",
            Self::LogbookAppend => "layover_logbook_append",
            Self::Status => "layover_status",
            Self::Wait => "layover_wait",
        }
    }

    /// What it is for, written for the agent that will read it.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Send => {
                "Send work to another agent. This is the only way work moves, and sending is what \
                 starts the agent you send to -- there is no separate spawn. You may only send to \
                 agents `layover_peers` lists."
            }
            Self::Peers => {
                "List the agents you may send to and what each one is for. Call this before \
                 deciding where work goes rather than guessing at names."
            }
            Self::Report => {
                "Say what you concluded. One headline, then the detail. Write it for somebody who \
                 was not watching and will read only the headline -- this is the account of your \
                 run that survives."
            }
            Self::Help => {
                "Say that something is in the way and you could not get past it. Use this instead \
                 of producing a plausible answer you do not believe: a wrong answer nobody flags \
                 travels downstream, and that is the failure this exists to prevent."
            }
            Self::MemoryRead => {
                "Read your own notes in full. Every run starts fresh, so this is the only thing \
                 you remember. The most recent part is already in your instructions."
            }
            Self::MemoryWrite => {
                "Add to your own notes, for future runs of you. Nothing else carries over, so \
                 anything worth remembering has to be written here deliberately."
            }
            Self::Learn => {
                "Propose something future runs of you should know. It applies immediately and \
                 lapses unless later runs arrive at it independently."
            }
            Self::LogbookAppend => {
                "Add to the factory's shared memory, which every agent can read. For things the \
                 whole factory needs, not for your own notes."
            }
            Self::Status => {
                "Ask what this chain has left: how many more messages it may send, and how much \
                 budget remains. Worth checking before fanning out to several agents."
            }
            Self::Wait => {
                "Set this work down and have it picked up later, when something you are waiting \
                 for has happened. Nothing is kept running in the meantime."
            }
        }
    }

    /// Finds a tool by the name an agent would call.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.name() == name)
    }

    /// Whether this tool changes anything outside the run that called it.
    ///
    /// Used to decide what a read-only agent may do: an agent given a worktree snapshot so it
    /// cannot disturb anyone should not be able to disturb anyone through a tool either.
    #[must_use]
    pub const fn writes(self) -> bool {
        matches!(
            self,
            Self::Send | Self::MemoryWrite | Self::LogbookAppend | Self::Learn | Self::Wait
        )
    }
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Every `layover_*` name mentioned in `text` that is not a tool.
///
/// Used by validation against prompt files. Deliberately anchored on the `layover_` prefix: an
/// agent told to call `layover_publish` has been told something false, whereas an agent told to
/// call `git` has been told something this crate knows nothing about.
#[must_use]
pub fn unknown_tools_in(text: &str) -> Vec<String> {
    let mut found = Vec::new();

    for (index, _) in text.match_indices("layover_") {
        let rest = &text[index..];
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let name = &rest[..end];

        if Tool::from_name(name).is_none() && !found.iter().any(|seen| seen == name) {
            found.push(name.to_owned());
        }
    }

    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_distinct_name() {
        let mut names: Vec<_> = Tool::ALL.iter().map(|tool| tool.name()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();

        assert_eq!(names.len(), before, "two tools answer to the same name");
    }

    #[test]
    fn every_name_round_trips() {
        for tool in Tool::ALL {
            assert_eq!(Tool::from_name(tool.name()), Some(tool));
        }
    }

    #[test]
    fn every_tool_is_named_for_layover() {
        // An agent's tool list mixes ours with the CLI's own and any MCP server the factory
        // declared. A shared prefix is what makes ours identifiable at a glance.
        for tool in Tool::ALL {
            assert!(tool.name().starts_with("layover_"), "{}", tool.name());
        }
    }

    #[test]
    fn every_tool_says_what_it_is_for() {
        // The description is what an agent reads when deciding whether to call it. A tool with a
        // thin description gets used wrongly or not at all.
        for tool in Tool::ALL {
            assert!(
                tool.description().len() > 60,
                "`{}` needs a description an agent can act on",
                tool.name()
            );
        }
    }

    #[test]
    fn there_is_no_spawn_tool() {
        // A `mode = "spawn"` route already opens an itinerary per flight. A tool doing the same
        // would be a second permission model over one graph.
        assert_eq!(Tool::from_name("layover_spawn"), None);
    }

    #[test]
    fn a_prompt_naming_a_tool_that_does_not_exist_is_caught() {
        let prompt = "Investigate, then call layover_publish to ship it.";
        assert_eq!(unknown_tools_in(prompt), vec!["layover_publish"]);
    }

    #[test]
    fn a_prompt_naming_real_tools_is_clean() {
        let prompt = "Call layover_peers, then layover_send. If stuck, layover_help.";
        assert!(unknown_tools_in(prompt).is_empty());
    }

    #[test]
    fn the_same_wrong_name_twice_is_reported_once() {
        let prompt = "use layover_ship. then layover_ship again.";
        assert_eq!(unknown_tools_in(prompt), vec!["layover_ship"]);
    }

    #[test]
    fn a_tool_name_followed_by_punctuation_is_still_recognised() {
        for prompt in [
            "call layover_send(...)",
            "call `layover_send`",
            "call layover_send.",
            "call layover_send, then wait",
        ] {
            assert!(unknown_tools_in(prompt).is_empty(), "{prompt}");
        }
    }

    #[test]
    fn a_read_only_agent_can_be_told_which_tools_would_change_things() {
        assert!(Tool::Send.writes());
        assert!(Tool::MemoryWrite.writes());
        assert!(!Tool::Peers.writes());
        assert!(!Tool::MemoryRead.writes());
        assert!(
            !Tool::Report.writes(),
            "a report is an account of a run, not an effect on anything else"
        );
    }
}
