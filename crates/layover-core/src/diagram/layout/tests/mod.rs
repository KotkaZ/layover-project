use super::*;
use crate::diagram::Activity;

mod geometry;
mod reading;
mod returns;

fn config(body: &str) -> Config {
    Config::from_toml(body, "layout-test.toml").expect("config parses")
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

        [agents.developer]
        prompt = "develop"

        [agents.tester]
        prompt = "test"
        access = "read-only"

        [agents.publisher]
        prompt = "publish"

        [pipelines.triage]
        entry = "analyst"

        [[routes]]
        from = "analyst"
        to = "developer"

        [[routes]]
        from = "developer"
        to = "tester"

        [[routes]]
        from = "tester"
        to = "developer"
        join = "all"

        [[routes]]
        from = "developer"
        to = "publisher"
        "#,
    )
}

#[test]
fn work_flows_left_to_right_in_reading_order() {
    // Columns are where an agent comes in the work: depth-first from the way in, a route that
    // closes a loop set aside. Here that is also one column per hop — the tester's verdict back to
    // the developer is the loop — but it need not be; see `reading`.
    let layout = Layout::build(&factory(), &Live::default());

    assert_eq!(layout.node("p_triage").expect("pipeline").layer, 0);
    assert_eq!(layout.node("a_analyst").expect("analyst").layer, 1);
    assert_eq!(layout.node("a_developer").expect("developer").layer, 2);
    assert_eq!(layout.node("a_tester").expect("tester").layer, 3);
}

#[test]
fn a_loop_does_not_hang_the_layout() {
    // Longest-path layering, the textbook approach, does not terminate on a cycle, and here the
    // developer and the tester point at each other — the review loop the reference factory is
    // built around. Setting aside each route that closes a loop, as the depth-first walk finds
    // it, is what leaves a graph longest path can finish on.
    let layout = Layout::build(&factory(), &Live::default());

    assert_eq!(layout.nodes.len(), 5);
    assert!(layout.width > 0.0 && layout.height > 0.0);
}

#[test]
fn a_sender_the_barrier_does_not_name_is_marked_as_bypassing_it() {
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

    let layout = Layout::build(&config, &Live::default());
    let bypass = layout
        .edges
        .iter()
        .find(|edge| edge.from == "a_scanner" && edge.to == "a_collector")
        .expect("scanner may send to collector");

    assert_eq!(bypass.style, EdgeStyle::Bypass);
    assert_eq!(
        bypass.label, None,
        "it does not wait, so it has no condition"
    );
}

#[test]
fn a_joined_agent_is_drawn_as_a_gate() {
    let layout = Layout::build(&factory(), &Live::default());

    assert_eq!(
        layout.node("a_developer").expect("developer").shape,
        Shape::Gate
    );
    assert_eq!(layout.node("a_analyst").expect("analyst").shape, Shape::Box);
}

#[test]
fn nodes_in_a_column_never_overlap() {
    let layout = Layout::build(&factory(), &Live::default());

    for layer in 0..4 {
        let mut boxes: Vec<(f64, f64)> = layout
            .nodes
            .iter()
            .filter(|node| node.layer == layer)
            .map(|node| (node.y, node.y + node.h))
            .collect();
        boxes.sort_by(|a, b| a.0.total_cmp(&b.0));

        for pair in boxes.windows(2) {
            assert!(
                pair[1].0 >= pair[0].1,
                "layer {layer} has overlapping nodes: {pair:?}"
            );
        }
    }
}

#[test]
fn every_node_sits_inside_the_reported_extent() {
    // The extent becomes the SVG viewBox. A node outside it is a node nobody can see.
    let layout = Layout::build(&factory(), &Live::default());

    for node in &layout.nodes {
        assert!(node.x >= 0.0 && node.y >= 0.0, "{} is off-canvas", node.id);
        assert!(node.x + node.w <= layout.width, "{} overflows", node.id);
        assert!(node.y + node.h <= layout.height, "{} overflows", node.id);
    }
}

/// A developer fanning out to a tester and a reviewer, both reporting back. The commonest
/// shape in a real factory and the one that exposed the shared-gutter problem.
fn review_loop() -> Config {
    config(
        r#"
        [layover]
        work_dir = "work"

        [defaults]
        runner = "claude"

        [runners.claude]
        command = ["claude", "-p"]

        [agents.dev]
        prompt = "develop"

        [agents.tester]
        prompt = "test"

        [agents.reviewer]
        prompt = "review"

        [pipelines.go]
        entry = "dev"

        [[routes]]
        from = "dev"
        to = ["tester", "reviewer"]

        [[routes]]
        from = ["tester", "reviewer"]
        to = "dev"
        join = "all"
        "#,
    )
}

#[test]
fn a_workflow_is_laid_out_from_its_own_entry_only() {
    // Seeding every pipeline entry regardless of scope put agents that happen to be another
    // pipeline's way in near the left edge of a diagram they are late in. In the reference
    // factory the follower is reached through the publisher but is also the follow-up
    // pipeline's entry, so it landed in column two with an edge sweeping back across the
    // whole drawing.
    let config = config(
        r#"
        [layover]
        work_dir = "work"

        [defaults]
        runner = "claude"

        [runners.claude]
        command = ["claude", "-p"]

        [agents.analyst]
        prompt = "analyse"

        [agents.publisher]
        prompt = "publish"

        [agents.follower]
        prompt = "follow"

        [pipelines.triage]
        entry = "analyst"

        [pipelines.follow_up]
        entry = "follower"

        [[routes]]
        from = "analyst"
        to = "publisher"

        [[routes]]
        from = "publisher"
        to = "follower"
        "#,
    );

    let triage = Layout::scoped(&config, &Live::default(), &Scope::Pipeline("triage".into()));
    let publisher = triage.node("a_publisher").expect("publisher");
    let follower = triage.node("a_follower").expect("follower");

    assert!(
        follower.layer > publisher.layer,
        "the follower is reached through the publisher here, so it must come after it: \
         publisher at {}, follower at {}",
        publisher.layer,
        follower.layer
    );
}

#[test]
fn a_workflow_can_be_drawn_on_its_own() {
    // A factory holds several pipelines and they are genuinely separate workflows. Drawing
    // them together produces one tangle that reads as a single very confused process, which
    // is exactly what a reader concludes from it.
    let config = two_workflows();
    let sweep = Layout::scoped(&config, &Live::default(), &Scope::Pipeline("sweep".into()));

    assert!(sweep.node("p_sweep").is_some());
    assert!(sweep.node("a_sweeper").is_some());
    assert!(
        sweep.node("a_pr_reviewer").is_some(),
        "a spawned reviewer is part of the sweep"
    );
    assert!(sweep.node("p_build").is_none(), "the other way in is not");
    assert!(sweep.node("a_developer").is_none());
}

#[test]
fn an_agent_in_two_workflows_appears_in_both() {
    // The honest answer. The developer really is in both pipelines, and hiding it from one
    // would misrepresent the factory to make a tidier picture.
    let config = two_workflows();

    for pipeline in ["build", "release"] {
        let drawn = Layout::scoped(&config, &Live::default(), &Scope::Pipeline(pipeline.into()));
        assert!(
            drawn.node("a_developer").is_some(),
            "developer missing from {pipeline}"
        );
    }
}

#[test]
fn drawing_everything_is_still_the_default() {
    let config = two_workflows();
    let all = Layout::build(&config, &Live::default());

    assert!(all.node("p_sweep").is_some());
    assert!(all.node("p_build").is_some());
    assert!(all.node("a_pr_reviewer").is_some());
}

fn two_workflows() -> Config {
    config(
        r#"
        [layover]
        work_dir = "work"

        [defaults]
        runner = "claude"

        [runners.claude]
        command = ["claude", "-p"]

        [agents.sweeper]
        prompt = "sweep"

        [agents.pr_reviewer]
        prompt = "review one"

        [agents.developer]
        prompt = "develop"

        [agents.publisher]
        prompt = "publish"

        [pipelines.sweep]
        entry = "sweeper"

        [pipelines.build]
        entry = "developer"

        [pipelines.release]
        entry = "developer"

        [[routes]]
        from = "sweeper"
        to = "pr_reviewer"
        mode = "spawn"

        [[routes]]
        from = "developer"
        to = "publisher"
        "#,
    )
}

#[test]
fn a_spawn_target_is_drawn_right_of_the_agent_that_spawns_it() {
    // Hop arithmetic stops at a spawn edge, which is right for hops and wrong for a picture: when
    // columns were hop distance it left the target at column zero beside its spawner, and the
    // edge between them became a same-column loop that read as a stray squiggle. Found by
    // standing up a real factory and looking at the dashboard. The depth-first walk takes a spawn
    // like any other route, so the target sits right of the agent that spawns it.
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

        [agents.pr_reviewer]
        prompt = "review one"

        [pipelines.sweep]
        entry = "sweeper"

        [[routes]]
        from = "sweeper"
        to = "pr_reviewer"
        mode = "spawn"
        "#,
    );

    let layout = Layout::build(&config, &Live::default());
    let sweeper = layout.node("a_sweeper").expect("sweeper");
    let reviewer = layout.node("a_pr_reviewer").expect("reviewer");

    assert!(
        reviewer.layer > sweeper.layer,
        "a spawn should still flow rightwards: sweeper at {}, reviewer at {}",
        sweeper.layer,
        reviewer.layer
    );

    let edge = layout
        .edges
        .iter()
        .find(|edge| edge.to == "a_pr_reviewer")
        .expect("the spawn edge exists");
    assert!(!edge.back, "and must not be drawn as a return path");
}

#[test]
fn an_agent_no_pipeline_can_reach_is_still_drawn() {
    // An unreachable agent is exactly what somebody opens the diagram to find, so dropping
    // it would defeat the purpose of drawing one.
    let config = config(
        r#"
        [layover]
        work_dir = "work"

        [defaults]
        runner = "claude"

        [runners.claude]
        command = ["claude", "-p"]

        [agents.reachable]
        prompt = "work"

        [agents.orphan]
        prompt = "nobody routes here"

        [pipelines.go]
        entry = "reachable"
        "#,
    );

    let layout = Layout::build(&config, &Live::default());

    assert!(layout.node("a_orphan").is_some());
}

#[test]
fn live_state_lands_on_the_right_node() {
    let live = Live::default().with("developer", Activity::Running);
    let layout = Layout::build(&factory(), &live);

    assert_eq!(
        layout.node("a_developer").expect("developer").activity,
        Some(Activity::Running)
    );
    assert_eq!(layout.node("a_analyst").expect("analyst").activity, None);
}

#[test]
fn an_empty_factory_lays_out_without_panicking() {
    let config = config(
        r#"
        [layover]
        work_dir = "work"

        [defaults]
        runner = "claude"

        [runners.claude]
        command = ["claude", "-p"]

        [agents.only]
        prompt = "think"
        entry = true
        "#,
    );

    let layout = Layout::build(&config, &Live::default());

    assert_eq!(layout.nodes.len(), 1);
    assert!(layout.edges.is_empty());
}
