//! Drawing a factory.
//!
//! Two renderers, deliberately, because they answer different questions.
//!
//! - [`mermaid`] emits Mermaid source. It is for *portability*: paste it into a README, an issue
//!   or a chat window and something will draw it. `layover graph` prints this.
//! - [`layout`] and [`svg`] draw the graph directly. That is for the *dashboard*, where the
//!   diagram has to carry live state, respond to a pointer, and load instantly.
//!
//! The alternative was to use Mermaid for both, and it was rejected on size: the Mermaid runtime
//! is 2.5 MB of JavaScript, which would have to be vendored into the repository and embedded in
//! the binary to keep the dashboard working offline. That is a poor trade for laying out twenty
//! nodes, and a layered layout for a graph this small is a few hundred lines that can actually be
//! unit-tested — which asserting on a JavaScript library's output could not be.

mod caption;
pub mod layout;
pub mod mermaid;
mod overlay;
pub mod svg;

use std::collections::{BTreeMap, BTreeSet};

use crate::agent::AgentName;

pub use layout::{Edge, EdgeStyle, Layout, Node, NodeKind, Shape};
pub use mermaid::{route_map, route_map_for};
pub use overlay::{Leg, Sender, Tally};
pub use svg::render as render_svg;

/// Which workflow to draw.
///
/// A factory holds several pipelines and they are genuinely separate workflows: a nightly sweep
/// has nothing to do with taking a work item to a pull request. Drawing them together produces one
/// tangle that reads as a single, very confused process — which is what a reader concludes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Scope {
    /// Every pipeline and every route in the factory.
    #[default]
    Everything,
    /// Only what this pipeline sets in motion.
    Pipeline(crate::pipeline::PipelineName),
}

impl Scope {
    /// The pipeline being drawn, or `None` for the whole factory.
    #[must_use]
    pub fn pipeline(&self) -> Option<&crate::pipeline::PipelineName> {
        match self {
            Self::Everything => None,
            Self::Pipeline(name) => Some(name),
        }
    }
}

/// What an agent is doing right now, for colouring a diagram.
///
/// Absent from the map means idle. Idle is the overwhelmingly common state, so storing it would
/// be storing mostly nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Activity {
    /// At least one run of this agent is in flight.
    Running,
    /// Flights are parked at this agent's barrier, waiting for the rest.
    Waiting,
    /// This agent's last run ended badly.
    Failed,
    /// On one chain's map: this agent ran in the chain, and its last run there went well.
    Done,
    /// On one chain's map: work for this agent is queued, waiting for a free slot.
    Queued,
}

impl Activity {
    /// The class name used to colour a node in this state.
    #[must_use]
    pub fn class(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Failed => "failed",
            Self::Done => "done",
            Self::Queued => "queued",
        }
    }
}

/// How the factory is currently behaving, overlaid on the static route map.
///
/// Empty by default, which renders the plain topology. That matters because the diagram has to
/// work before the Tower has ever run anything.
#[derive(Debug, Clone, Default)]
pub struct Live {
    /// What each busy agent is doing.
    pub activity: BTreeMap<AgentName, Activity>,
    /// How many runs to count on each agent: on one chain's map, how often it ran there; on a
    /// workflow's, how many of its runs are alive at once.
    pub tally: BTreeMap<AgentName, Tally>,
    /// On one chain's map, the routes its work took.
    pub travelled: BTreeSet<Leg>,
}

impl Live {
    /// Marks an agent as being in a state.
    #[must_use]
    pub fn with(mut self, agent: impl Into<AgentName>, activity: Activity) -> Self {
        self.activity.insert(agent.into(), activity);
        self
    }

    /// Counts runs on an agent, saying over what: `"in this chain"`, `"alive now"`.
    #[must_use]
    pub fn counted(mut self, agent: impl Into<AgentName>, runs: usize, of: &'static str) -> Self {
        self.tally.insert(agent.into(), Tally { runs, of });
        self
    }

    /// Records that work went from `from` to `to`.
    #[must_use]
    pub fn travelled(mut self, from: Sender, to: impl Into<AgentName>) -> Self {
        self.travelled.insert(Leg {
            from,
            to: to.into(),
        });
        self
    }

    /// Returns `true` when nothing is happening.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.activity.is_empty()
    }
}
