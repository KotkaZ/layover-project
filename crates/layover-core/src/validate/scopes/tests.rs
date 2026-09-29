//! What the scope checks accept and refuse.

use crate::validate::testing::{assert_mentions, errors, parse, warnings};
use crate::validate::validate;

const AGENTS: &str = r#"
    [reserve]
    fuel_usd = 100.0

    [agents.analyst]
    runner = "claude"
    description = "analyses"
    prompt = "a"
    access = "read-only"

    [agents.bob]
    runner = "claude"
    description = "builds"
    prompt = "b"

    [agents.azurix]
    runner = "claude"
    description = "posts"
    prompt = "z"
    access = "read-only"

    [agents.eagle]
    runner = "claude"
    description = "reviews"
    prompt = "e"
    access = "read-only"

    [pipelines.devforge]
    entry = "analyst"

    [pipelines.eagle-eye]
    entry = "azurix"
"#;

fn with_routes(routes: &str) -> crate::config::Config {
    parse(&format!("{AGENTS}\n{routes}"))
}

#[test]
fn a_scoped_factory_that_holds_together_is_clean() {
    let config = with_routes(
        r#"
        [[routes]]
        from = "analyst"
        to = ["bob", "eagle"]
        pipelines = "devforge"

        [[routes]]
        from = "eagle"
        to = "bob"
        pipelines = "devforge"

        [[routes]]
        from = "azurix"
        to = "eagle"
        mode = "spawn"
        pipelines = "eagle-eye"

        [[routes]]
        from = "eagle"
        to = "azurix"
        pipelines = "eagle-eye"
        "#,
    );

    assert_eq!(validate(&config), Vec::new());
}

#[test]
fn a_scope_naming_an_unknown_pipeline_is_an_error() {
    let config = with_routes(
        r#"
        [[routes]]
        from = "analyst"
        to = "bob"
        pipelines = ["devforge", "devfroge"]
        "#,
    );

    assert_mentions(&errors(&config), "scoped to unknown pipeline `devfroge`");
}

#[test]
fn an_empty_scope_is_an_error_rather_than_a_guess() {
    let config = with_routes(
        r#"
        [[routes]]
        from = "analyst"
        to = "bob"
        pipelines = []
        "#,
    );

    assert_mentions(&errors(&config), "`pipelines = []`");
}

#[test]
fn overlapping_routes_that_disagree_about_spawning_are_an_error() {
    let config = with_routes(
        r#"
        [[routes]]
        from = "analyst"
        to = "bob"

        [[routes]]
        from = "azurix"
        to = "eagle"
        mode = "spawn"
        pipelines = "eagle-eye"

        [[routes]]
        from = "azurix"
        to = "eagle"
        pipelines = ["eagle-eye", "devforge"]
        "#,
    );

    assert_mentions(&errors(&config), "only one of them spawns");
    assert_mentions(&errors(&config), "`eagle-eye`");
}

#[test]
fn routes_in_separate_scopes_may_treat_one_pair_differently() {
    // The point of a scope: one workflow spawns a review per item, another hands work on.
    let config = with_routes(
        r#"
        [[routes]]
        from = "analyst"
        to = "azurix"
        pipelines = "devforge"

        [[routes]]
        from = "azurix"
        to = "eagle"
        mode = "spawn"
        pipelines = "eagle-eye"

        [[routes]]
        from = "azurix"
        to = "eagle"
        pipelines = "devforge"
        "#,
    );

    assert!(
        !errors(&config).iter().any(|m| m.contains("spawns")),
        "{:?}",
        errors(&config)
    );
}

#[test]
fn a_disagreement_between_two_global_routes_is_left_as_it_was() {
    // Backward compatibility: this ambiguity predates scopes, and reporting it now would change
    // what an existing factory is told.
    let config = with_routes(
        r#"
        [[routes]]
        from = "azurix"
        to = "eagle"
        mode = "spawn"

        [[routes]]
        from = "azurix"
        to = "eagle"
        "#,
    );

    assert!(
        !errors(&config).iter().any(|m| m.contains("spawns")),
        "{:?}",
        errors(&config)
    );
}

#[test]
fn a_spawn_into_a_barrier_another_route_declares_in_the_same_scope_is_an_error() {
    let config = with_routes(
        r#"
        [[routes]]
        from = "analyst"
        to = "azurix"

        [[routes]]
        from = "azurix"
        to = "eagle"
        mode = "spawn"
        pipelines = "eagle-eye"

        [[routes]]
        from = ["azurix", "bob"]
        to = "eagle"
        join = "all"
        "#,
    );

    assert_mentions(&errors(&config), "reach the barrier alone");
}

#[test]
fn a_scoped_route_no_chain_of_its_scope_can_use_warns() {
    // `bob` is only ever woken in DevForge, so an Eagle Eye route out of `bob` is dead.
    let config = with_routes(
        r#"
        [[routes]]
        from = "analyst"
        to = "bob"
        pipelines = "devforge"

        [[routes]]
        from = "azurix"
        to = "eagle"
        pipelines = "eagle-eye"

        [[routes]]
        from = "bob"
        to = "eagle"
        pipelines = "eagle-eye"
        "#,
    );

    assert_mentions(&warnings(&config), "route 2 is scoped to `eagle-eye`");
    assert_mentions(&warnings(&config), "can wake `bob`");
}

#[test]
fn a_resuming_pipeline_can_use_a_route_from_any_agent_that_could_book_a_layover() {
    // Resumed work goes back to whoever set it down, not to `entry`. `bob` is woken in
    // DevForge and may book a layover, so a follow-up route out of `bob` is live.
    let config = with_routes(
        r#"
        [pipelines.follow-up]
        entry = "azurix"
        trigger = { every = "1h" }
        resumes = true

        [[routes]]
        from = "analyst"
        to = "bob"
        pipelines = "devforge"

        [[routes]]
        from = "azurix"
        to = "eagle"
        pipelines = "eagle-eye"

        [[routes]]
        from = "bob"
        to = "analyst"
        pipelines = "follow-up"
        "#,
    );

    assert!(
        !warnings(&config)
            .iter()
            .any(|m| m.contains("nothing can use it")),
        "{:?}",
        warnings(&config)
    );
}

#[test]
fn an_entry_agent_whose_routes_are_all_scoped_is_stranded_by_a_direct_trigger() {
    let config = parse(
        r#"
        [agents.scanner]
        runner = "claude"
        description = "scans"
        prompt = "s"
        entry = true
        access = "read-only"

        [agents.reviewer]
        runner = "claude"
        description = "reviews"
        prompt = "r"
        access = "read-only"

        [pipelines.sweep]
        entry = "scanner"

        [[routes]]
        from = "scanner"
        to = "reviewer"
        pipelines = "sweep"
        "#,
    );

    assert_mentions(
        &warnings(&config),
        "agent `scanner` is marked `entry = true`",
    );
}

#[test]
fn an_entry_agent_with_one_global_route_out_is_not_stranded() {
    let config = parse(
        r#"
        [agents.scanner]
        runner = "claude"
        description = "scans"
        prompt = "s"
        entry = true
        access = "read-only"

        [agents.reviewer]
        runner = "claude"
        description = "reviews"
        prompt = "r"
        access = "read-only"

        [pipelines.sweep]
        entry = "scanner"

        [[routes]]
        from = "scanner"
        to = "reviewer"
        pipelines = "sweep"

        [[routes]]
        from = "scanner"
        to = "reviewer"
        "#,
    );

    assert!(
        !warnings(&config).iter().any(|m| m.contains("entry = true")),
        "{:?}",
        warnings(&config)
    );
}
