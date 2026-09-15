//! Guards against the documented configurations drifting away from the parser.
//!
//! `docs/architecture.md` shows a factory definition. If that shape stops loading, an agent
//! following the documentation would produce a configuration Layover rejects, and the
//! documentation would be actively misleading rather than merely stale.

use std::path::{Path, PathBuf};

use layover_core::{Config, Trigger, validate};

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

#[test]
fn the_documented_example_parses_and_validates() {
    let path = examples_dir().join("planner.toml");

    let config = Config::load(&path).expect("the documented example must remain loadable");

    assert_eq!(
        validate(&config),
        Vec::new(),
        "the documented example must validate cleanly"
    );
    assert_eq!(config.agents.len(), 3);
    assert_eq!(config.runners.len(), 3);
    assert_eq!(config.entry_agents().count(), 1);
}

#[test]
fn the_documented_example_declares_a_manual_pipeline() {
    let config = Config::load(examples_dir().join("planner.toml")).expect("example loads");

    let pipeline = config
        .pipelines
        .get(&"build".into())
        .expect("the example declares a `build` pipeline");

    assert_eq!(pipeline.trigger, Trigger::Manual);
    assert_eq!(pipeline.entry.as_str(), "planner");
    assert_eq!(config.scheduled_pipelines().count(), 0);
}

#[test]
fn every_example_agent_says_what_it_is_for() {
    // `layover_peers()` hands these to running agents. An undescribed agent is a bare name.
    for example in ["planner.toml", "workitem-factory/layover.toml"] {
        let config = Config::load(examples_dir().join(example)).expect("example loads");

        for (name, agent) in &config.agents {
            assert!(
                agent.description.is_some(),
                "`{name}` in {example} has no description"
            );
        }
    }
}
