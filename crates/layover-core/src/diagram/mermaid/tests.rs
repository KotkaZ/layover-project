//! What the Mermaid route map says, and how.

use super::*;

fn config(body: &str) -> Config {
    Config::from_toml(body, "diagram-test.toml").expect("config parses")
}

fn factory() -> Config {
    config(
        r#"
        [layover]
        work_dir = "work"

        [defaults]
        runner = "claude"

        [runners.claude]
        command = ["claude", "-p"]

        [agents.analyst]
        prompt = "analyse"

        [agents.investigator]
        prompt = "investigate"
        access = "read-only"

        [agents.developer]
        prompt = "develop"

        [pipelines.triage]
        entry = "analyst"

        [[routes]]
        from = "analyst"
        to = "investigator"

        [[routes]]
        from = ["analyst", "investigator"]
        to = "developer"
        join = "all"
        "#,
    )
}

#[test]
fn every_agent_and_route_reaches_the_diagram() {
    let mermaid = route_map(&factory(), &Live::default());

    assert!(mermaid.starts_with("flowchart LR\n"));
    for agent in ["a_analyst", "a_investigator", "a_developer"] {
        assert!(mermaid.contains(agent), "{agent} missing from\n{mermaid}");
    }
    assert!(mermaid.contains("a_analyst --> a_investigator"));
}

#[test]
fn a_pipeline_is_drawn_as_the_way_in_rather_than_as_an_agent() {
    // "How does work get in" is the first question a diagram like this has to answer, and
    // the route map on its own cannot: an entry agent looks like any other node.
    let mermaid = route_map(&factory(), &Live::default());

    assert!(mermaid.contains(r#"subgraph pipelines ["Ways in"]"#));
    assert!(mermaid.contains("p_triage ==> a_analyst"));
    assert!(mermaid.contains("class p_triage pipeline;"));
}

#[test]
fn a_barrier_is_drawn_as_a_gate_with_its_condition_on_the_arrows() {
    let mermaid = route_map(&factory(), &Live::default());

    assert!(
        mermaid.contains(r#"a_developer{{"developer"}}"#),
        "a joined agent should be a gate, not a box:\n{mermaid}"
    );
    assert!(mermaid.contains("a_analyst -- all --> a_developer"));
    assert!(mermaid.contains("a_investigator -- all --> a_developer"));
}

#[test]
fn read_only_access_is_visible_on_the_node() {
    let mermaid = route_map(&factory(), &Live::default());

    assert!(mermaid.contains("investigator<br/><small>read-only</small>"));
}

#[test]
fn an_idle_factory_emits_no_state_colours() {
    // The diagram has to work before the Tower has ever run anything, and a palette of
    // unused classes is three lines of noise in something meant to be readable as text.
    let mermaid = route_map(&factory(), &Live::default());

    assert!(!mermaid.contains("classDef running"));
    assert!(!mermaid.contains("classDef waiting"));
    assert!(!mermaid.contains("classDef failed"));
}

#[test]
fn live_state_colours_only_the_agents_in_it() {
    let live = Live::default()
        .with("developer", Activity::Running)
        .with("analyst", Activity::Failed);

    let mermaid = route_map(&factory(), &live);

    assert!(mermaid.contains("class a_developer running;"));
    assert!(mermaid.contains("class a_analyst failed;"));
    assert!(
        !mermaid.contains("classDef waiting"),
        "nothing is waiting, so no class for it"
    );
}

#[test]
fn a_sender_the_barrier_does_not_name_is_drawn_as_bypassing_it() {
    // A barrier constrains only the upstreams it names; any other permitted sender wakes the
    // agent directly. Labelling that edge with the join condition would state the opposite of
    // what happens, which is exactly the misreading that makes a joined agent look like it
    // can never also be an entry point. Found by generating the reference factory and
    // looking at it.
    let config = config(
        r#"
        [layover]
        work_dir = "work"

        [defaults]
        runner = "claude"

        [runners.claude]
        command = ["claude", "-p"]

        [agents.scanner]
        prompt = "scan"

        [agents.left]
        prompt = "left"

        [agents.right]
        prompt = "right"

        [agents.collector]
        prompt = "collect"

        [pipelines.go]
        entry = "scanner"

        [[routes]]
        from = ["left", "right"]
        to = "collector"
        join = "all"

        [[routes]]
        from = "scanner"
        to = "collector"
        "#,
    );

    let mermaid = route_map(&config, &Live::default());

    assert!(mermaid.contains("a_left -- all --> a_collector"));
    assert!(mermaid.contains("a_right -- all --> a_collector"));
    assert!(
        mermaid.contains("a_scanner -. bypasses .-> a_collector"),
        "scanner is not one of the barrier's upstreams:\n{mermaid}"
    );
    assert!(!mermaid.contains("a_scanner -- all"));
}

#[test]
fn a_repeated_edge_is_drawn_once() {
    // Two routes may name the same pair — a fan-out and a rendezvous can overlap — and
    // Mermaid would draw two parallel arrows for it.
    let config = config(
        r#"
        [layover]
        work_dir = "work"

        [defaults]
        runner = "claude"

        [runners.claude]
        command = ["claude", "-p"]

        [agents.a]
        prompt = "a"

        [agents.b]
        prompt = "b"

        [pipelines.go]
        entry = "a"

        [[routes]]
        from = "a"
        to = "b"

        [[routes]]
        from = "a"
        to = "b"
        "#,
    );

    let mermaid = route_map(&config, &Live::default());

    assert_eq!(mermaid.matches("a_a --> a_b").count(), 1);
}

#[test]
fn a_factory_with_no_pipelines_still_renders() {
    let config = config(
        r#"
        [layover]
        work_dir = "work"

        [defaults]
        runner = "claude"

        [runners.claude]
        command = ["claude", "-p"]

        [agents.lonely]
        prompt = "think"
        entry = true
        "#,
    );

    let mermaid = route_map(&config, &Live::default());

    assert!(mermaid.contains("a_lonely"));
    assert!(!mermaid.contains("subgraph"));
    assert!(!mermaid.contains("classDef pipeline"));
}

#[test]
fn the_trigger_is_shown_under_the_pipeline_name() {
    let config = config(
        r#"
        [layover]
        work_dir = "work"

        [defaults]
        runner = "claude"

        [runners.claude]
        command = ["claude", "-p"]

        [agents.sweeper]
        prompt = "sweep"

        [pipelines.nightly]
        entry = "sweeper"
        trigger = { cron = "0 3 * * *" }
        "#,
    );

    let mermaid = route_map(&config, &Live::default());

    assert!(mermaid.contains("nightly<br/><small>cron"), "{mermaid}");
}
