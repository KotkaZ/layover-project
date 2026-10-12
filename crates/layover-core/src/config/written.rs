//! `layover.toml` as written, and the factory it describes.
//!
//! A few keys are shorthand for something every part of Layover would otherwise have to work out
//! for itself: `[flags]` declares a flag for every workflow at once. They are resolved here, once,
//! as the file is read, so the rest of Layover sees each pipeline exactly as if it had declared
//! everything itself — and nothing downstream can forget to apply them.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use super::{Config, CopilotConfig, Defaults, LearningsConfig, Paths, ReserveConfig, Runner};
use crate::agent::{Agent, AgentName};
use crate::cost::RateCard;
use crate::pipeline::{FlagSpec, Pipeline, PipelineName};
use crate::route::Route;

/// The file, key for key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Written {
    #[serde(default)]
    layover: Paths,
    #[serde(default)]
    defaults: Defaults,
    #[serde(default)]
    reserve: ReserveConfig,
    #[serde(default)]
    rates: RateCard,
    #[serde(default)]
    copilot: CopilotConfig,
    #[serde(default)]
    learnings: LearningsConfig,
    #[serde(default)]
    flags: BTreeMap<String, FlagSpec>,
    #[serde(default)]
    runners: BTreeMap<String, Runner>,
    #[serde(default)]
    agents: BTreeMap<AgentName, Agent>,
    #[serde(default)]
    pipelines: BTreeMap<PipelineName, Pipeline>,
    #[serde(default)]
    routes: Vec<Route>,
}

impl From<Written> for Config {
    fn from(written: Written) -> Self {
        let mut pipelines = written.pipelines;
        for pipeline in pipelines.values_mut() {
            inherit_flags(pipeline, &written.flags);
        }

        Self {
            layover: written.layover,
            defaults: written.defaults,
            reserve: written.reserve,
            rates: written.rates,
            copilot: written.copilot,
            learnings: written.learnings,
            flags: written.flags,
            runners: written.runners,
            agents: written.agents,
            pipelines,
            routes: written.routes,
        }
    }
}

/// Gives `pipeline` every factory-wide flag it does not declare itself. Its own declaration wins,
/// so a workflow can start one shared flag from a different default.
fn inherit_flags(pipeline: &mut Pipeline, flags: &BTreeMap<String, FlagSpec>) {
    let mut inherited = BTreeSet::new();
    for (name, spec) in flags {
        if !pipeline.flags.contains_key(name) {
            pipeline.flags.insert(name.clone(), spec.clone());
            inherited.insert(name.clone());
        }
    }
    pipeline.inherited_flags = inherited;
}
