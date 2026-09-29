//! Guards against the documented configurations drifting away from the parser.
//!
//! `docs/architecture.md` shows a factory definition. If that shape stops loading, an agent
//! following the documentation would produce a configuration Layover rejects, and the
//! documentation would be actively misleading rather than merely stale.

use std::path::{Path, PathBuf};

use layover_core::{AgentName, Config, RouteMap, Trigger, validate};

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
    for example in [
        "planner.toml",
        "workitem-factory/layover.toml",
        "build-and-review/layover.toml",
    ] {
        let config = Config::load(examples_dir().join(example)).expect("example loads");

        for (name, agent) in &config.agents {
            assert!(
                agent.description.is_some(),
                "`{name}` in {example} has no description"
            );
        }
    }
}

#[test]
fn the_scoped_example_keeps_a_sweeps_reviewer_away_from_the_builder() {
    // What `build-and-review/` exists to show: one reviewer, two workflows, and only the build
    // workflow's chains may hand the builder work.
    let config =
        Config::load(examples_dir().join("build-and-review/layover.toml")).expect("example loads");
    let routes = RouteMap::from_config(&config);
    let (reviewer, builder) = (AgentName::from("reviewer"), AgentName::from("builder"));

    let sweep = routes.for_pipeline(Some(&"review-sweep".into()));
    assert!(!sweep.permits(&reviewer, &builder));
    assert!(!sweep.workflow_from(&"scanner".into()).contains(&builder));
    assert!(
        sweep.permits(&reviewer, &"notifier".into()),
        "global routes apply everywhere"
    );

    let build = routes.for_pipeline(Some(&"build".into()));
    assert!(build.permits(&reviewer, &builder));

    assert!(
        !routes.for_pipeline(None).permits(&reviewer, &builder),
        "a chain no pipeline started gets the global routes only"
    );
}
