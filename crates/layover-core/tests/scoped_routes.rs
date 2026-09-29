//! Pipeline-scoped routes, seen from outside the crate: what validation says about a factory that
//! scopes its routes.
//!
//! Unit tests beside the code pin each piece; these pin how the pieces combine, on factories
//! shaped like real ones.

mod common;

use common::scoped::{agents, config, karls_factory};
use layover_core::{Config, PromptMap, Severity, validate, validate_prompts};

fn findings(config: &Config, severity: Severity) -> Vec<String> {
    validate(config)
        .into_iter()
        .filter(|finding| finding.severity == severity)
        .map(|finding| finding.message)
        .collect()
}

#[test]
fn an_agent_only_another_pipelines_route_leads_to_cannot_be_reached() {
    // Before scopes, `b -> x` made `x` reachable because `b` is an entry. Scoped to `one`, it is
    // an edge only `one`'s chains may use, and `one` never wakes `b`.
    let config = config(&format!(
        r#"
        {}

        [pipelines.one]
        entry = "a"

        [pipelines.two]
        entry = "b"

        [[routes]]
        from = "b"
        to = "x"
        pipelines = "one"
        "#,
        agents(&["a", "b", "x"])
    ));

    let warnings = findings(&config, Severity::Warning);
    assert!(
        warnings.iter().any(|m| m.contains("`x` cannot be reached")),
        "{warnings:?}"
    );
}

#[test]
fn hop_depth_is_measured_in_the_pipeline_that_can_use_the_route() {
    // `c -> far` is a shortcut only `long`'s chains may take, and `long` reaches `far` through the
    // whole line. The shortcut's other end is `short`'s entry, which must not count.
    let config = config(&format!(
        r#"
        {}

        [defaults]
        max_hops = 3

        [pipelines.long]
        entry = "a"

        [pipelines.short]
        entry = "c"

        [[routes]]
        from = "a"
        to = "b"
        pipelines = "long"

        [[routes]]
        from = "b"
        to = "m"
        pipelines = "long"

        [[routes]]
        from = "m"
        to = "far"
        pipelines = "long"

        [[routes]]
        from = "c"
        to = "far"
        pipelines = "long"
        "#,
        agents(&["a", "b", "m", "c", "far"])
    ));

    let warnings = findings(&config, Severity::Warning);
    assert!(
        warnings.iter().any(|m| m.contains("`far` is 4 flights")),
        "{warnings:?}"
    );
}

#[test]
fn a_pipeline_need_not_declare_flags_for_agents_it_cannot_reach() {
    // The problem scoping exists to fix: a review sweep had to declare every flag the build
    // workflow's tester reads, because the one shared mesh let the sweep reach the tester.
    let config = config(&format!(
        r#"
        {}

        [agents.tester]
        runner = "claude"
        description = "tests"
        prompt_file = "tester.md"
        access = "read-only"

        [reserve]
        fuel_usd = 100.0

        [pipelines.build]
        entry = "analyst"

        [pipelines.build.flags]
        run_e2e = {{ default = false }}

        [pipelines.sweep]
        entry = "analyst"
        trigger = {{ every = "1h" }}

        [[routes]]
        from = "analyst"
        to = "tester"
        pipelines = "build"
        "#,
        agents(&["analyst"])
    ));
    let prompts = PromptMap::new()
        .with("tester.md", "Test.\n@include(run_e2e) e2e.md\n")
        .with("e2e.md", "Also the remote suite.\n");

    let found = validate_prompts(&config, &prompts);
    assert!(
        !found
            .iter()
            .any(|finding| finding.message.contains("run_e2e")),
        "{found:?}"
    );
}

#[test]
fn a_pipeline_that_can_reach_the_agent_still_has_to_declare_its_flags() {
    let config = config(&format!(
        r#"
        {}

        [agents.tester]
        runner = "claude"
        description = "tests"
        prompt_file = "tester.md"
        access = "read-only"

        [pipelines.build]
        entry = "analyst"

        [pipelines.sweep]
        entry = "analyst"

        [[routes]]
        from = "analyst"
        to = "tester"
        pipelines = ["build", "sweep"]
        "#,
        agents(&["analyst"])
    ));
    let prompts = PromptMap::new()
        .with("tester.md", "Test.\n@include(run_e2e) e2e.md\n")
        .with("e2e.md", "Also the remote suite.\n");

    let found = validate_prompts(&config, &prompts);
    assert!(
        found
            .iter()
            .any(|finding| finding.message.contains("pipeline `sweep`")),
        "{found:?}"
    );
}

#[test]
fn an_overlapping_schedule_only_warns_about_writers_its_own_routes_reach() {
    let config = config(&format!(
        r#"
        {}

        [agents.writer]
        runner = "claude"
        description = "writes"
        prompt = "w"

        [reserve]
        fuel_usd = 500.0

        [pipelines.build]
        entry = "scout"

        [pipelines.sweep]
        entry = "scout"
        trigger = {{ every = "1h" }}
        overlap = "allow"

        [[routes]]
        from = "scout"
        to = "writer"
        pipelines = "build"
        "#,
        agents(&["scout"])
    ));

    let warnings = findings(&config, Severity::Warning);
    assert!(
        !warnings
            .iter()
            .any(|m| m.contains("two instances would edit the same files")),
        "{warnings:?}"
    );
}

#[test]
fn two_joins_onto_one_agent_in_separate_pipelines_are_allowed_and_in_one_are_not() {
    let separate = config(&format!(
        r#"
        {}

        [pipelines.one]
        entry = "a"

        [pipelines.two]
        entry = "a"

        [[routes]]
        from = "a"
        to = ["x", "y"]

        [[routes]]
        from = ["x", "y"]
        to = "c"
        join = "all"
        pipelines = "one"

        [[routes]]
        from = ["x", "y"]
        to = "c"
        join = "any"
        pipelines = "two"
        "#,
        agents(&["a", "x", "y", "c"])
    ));
    let clashing = config(&format!(
        r#"
            {}

            [pipelines.one]
            entry = "a"

            [[routes]]
            from = "a"
            to = ["x", "y"]

            [[routes]]
            from = ["x", "y"]
            to = "c"
            join = "all"
            pipelines = "one"

            [[routes]]
            from = ["x", "y"]
            to = "c"
            join = "any"
            "#,
        agents(&["a", "x", "y", "c"])
    ));

    assert_eq!(findings(&separate, Severity::Error), Vec::<String>::new());
    let errors = findings(&clashing, Severity::Error);
    assert!(
        errors
            .iter()
            .any(|m| m.contains("routes 1 and 2") && m.contains("`one`")),
        "{errors:?}"
    );
}

#[test]
fn karls_scoped_factory_validates_clean() {
    let config = karls_factory();

    assert_eq!(validate(&config), Vec::new());
}
