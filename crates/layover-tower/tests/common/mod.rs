//! The factory the scoped-route tests share, and the helpers every one of them needs.
//!
//! It is Karl's case in miniature. `devforge` may hand `bob` work from `eagle`; `eagle-eye` spawns
//! an `eagle` per pull request and may not. `eagle` reads untrusted pull request text, so the route
//! map — not a prompt — has to be what keeps an Eagle Eye chain away from `bob`.
//!
//! Each test binary compiles this module separately and uses only part of it, so unused items are
//! expected rather than a sign of dead code.
#![allow(dead_code)]

use std::collections::BTreeMap;

use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;

/// The factory, running every agent with `runner`.
pub fn factory_text(runner: &str) -> String {
    format!(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "shell"
max_hops = 6
timeout_sec = 30

[runners.shell]
command = {runner}

[agents.analyst]
prompt = "analyse"

[agents.azurix]
prompt = "post"

[agents.eagle]
prompt = "review"

[agents.bob]
prompt = "build"

[agents.sherlock]
prompt = "investigate"

[agents.wolf]
prompt = "test"

[pipelines.devforge]
entry = "analyst"

[pipelines.follow-up]
entry = "azurix"
resumes = true

[pipelines.eagle-eye]
entry = "azurix"

[[routes]]
from = "analyst"
to = "eagle"
pipelines = ["devforge", "follow-up"]

[[routes]]
from = "eagle"
to = "bob"
pipelines = ["devforge", "follow-up"]

[[routes]]
from = "azurix"
to = "eagle"
mode = "spawn"
pipelines = "eagle-eye"

[[routes]]
from = "eagle"
to = "sherlock"

[[routes]]
from = ["sherlock", "wolf"]
to = "bob"
join = "all"
pipelines = "devforge"
"#
    )
}

/// A runner that succeeds at once, on either platform.
pub fn echo() -> &'static str {
    if cfg!(windows) {
        r#"["cmd", "/c", "echo done"]"#
    } else {
        r#"["sh", "-c", "echo done"]"#
    }
}

/// The factory, parsed.
pub fn config() -> Config {
    toml::from_str(&factory_text(r#"["echo"]"#)).expect("the fixture parses")
}

pub fn name(pipeline: &str) -> PipelineName {
    PipelineName::new(pipeline)
}

/// A flight an agent sent in `chain`, queued under `pipeline` as the Tower would record it.
pub fn queued(chain: &ItineraryId, from: &str, to: &str, pipeline: Option<&str>) -> Queued {
    Queued::new(
        Flight::new(
            chain.clone(),
            Origin::Agent(AgentName::new(from)),
            AgentName::new(to),
            "go",
            4,
        ),
        pipeline.map(name),
        BTreeMap::new(),
    )
}
